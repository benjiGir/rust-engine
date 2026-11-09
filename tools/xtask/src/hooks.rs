//! `cargo xtask hooks` : génère l'interface entre le jeu et les mods depuis le catalogue
//! `crates/mod_sdk/hooks.ron` (voir ce fichier). `--check` vérifie seulement que les
//! fichiers générés sont à jour (pour la CI).
//!
//! Idée reprise d'Horizon (`native-hooks.json`, qui génère l'API des mods, sa doc et ses
//! tests), réécrite ici pour nos mods WebAssembly.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

pub const CATALOG: &str = "crates/mod_sdk/hooks.ron";
const SDK_FFI: &str = "crates/mod_sdk/src/ffi.rs";
const HOST_ABI: &str = "crates/engine_mods/src/runtime/abi.rs";
const DOC: &str = "docs/modding.md";

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub enum Type {
    I32,
    I64,
    F32,
    F64,
}

impl Type {
    fn rust(self) -> &'static str {
        match self {
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Function {
    pub name: String,
    pub group: String,
    pub params: Vec<(String, Type)>,
    #[serde(default)]
    pub ret: Option<Type>,
    #[serde(default)]
    pub fallback: Option<String>,
    pub doc: String,
}

#[derive(Debug, Deserialize)]
pub struct Event {
    pub id: i32,
    pub name: String,
    pub constant: String,
    pub data: String,
    pub doc: String,
}

#[derive(Debug, Deserialize)]
pub struct MachineCall {
    pub id: i32,
    pub name: String,
    pub constant: String,
    pub doc: String,
}

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub module: String,
    pub functions: Vec<Function>,
    pub events: Vec<Event>,
    pub machine_calls: Vec<MachineCall>,
}

impl Catalog {
    pub fn parse(text: &str) -> Result<Self> {
        let catalog: Self = ron::from_str(text).context("lecture du catalogue")?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Les erreurs que le compilateur ne verrait qu'après coup, ou pas du tout.
    fn validate(&self) -> Result<()> {
        let mut names = std::collections::HashSet::new();
        for f in &self.functions {
            if !names.insert(&f.name) {
                bail!("la fonction « {} » est déclarée deux fois", f.name);
            }
            if f.ret.is_some() != f.fallback.is_some() {
                bail!(
                    "« {} » : `fallback` est requis avec `ret`, et seulement avec lui",
                    f.name
                );
            }
        }
        let mut ids = std::collections::HashSet::new();
        for e in &self.events {
            if e.id <= 0 || !ids.insert(e.id) {
                bail!(
                    "événement « {} » : numéro {} invalide ou en double",
                    e.name,
                    e.id
                );
            }
        }
        let mut ids = std::collections::HashSet::new();
        for m in &self.machine_calls {
            if m.id < 0 || !ids.insert(m.id) {
                bail!(
                    "appel « {} » : numéro {} invalide ou en double",
                    m.name,
                    m.id
                );
            }
        }
        Ok(())
    }
}

pub fn run(root: &Path, args: &[String]) -> Result<()> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => bail!("usage : cargo xtask hooks [--check]"),
    };
    let catalog = Catalog::parse(&std::fs::read_to_string(root.join(CATALOG))?)?;
    let outputs: Vec<(PathBuf, String)> = vec![
        (root.join(SDK_FFI), rustfmt(&sdk_ffi(&catalog))?),
        (root.join(HOST_ABI), rustfmt(&host_abi(&catalog))?),
        (
            root.join(DOC),
            doc(&std::fs::read_to_string(root.join(DOC))?, &catalog)?,
        ),
    ];
    let mut stale = Vec::new();
    for (path, content) in &outputs {
        let current = std::fs::read_to_string(path).unwrap_or_default();
        if current == *content {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        if check {
            stale.push(relative);
        } else {
            std::fs::write(path, content)?;
            println!("écrit : {relative}");
        }
    }
    if !stale.is_empty() {
        bail!(
            "pas à jour avec {CATALOG} : {} (lancer `cargo xtask hooks`)",
            stale.join(", ")
        );
    }
    println!(
        "interface des mods : {} fonctions, {} événements, {} appels de machines{}",
        catalog.functions.len(),
        catalog.events.len(),
        catalog.machine_calls.len(),
        if check { ", tout est à jour" } else { "" }
    );
    Ok(())
}

const HEADER: &str =
    "// GÉNÉRÉ par `cargo xtask hooks` depuis `crates/mod_sdk/hooks.ron` : ne pas modifier.\n";

fn constants(catalog: &Catalog, out: &mut String) {
    out.push_str("/// Les numéros des événements (`on_event`).\n");
    for e in &catalog.events {
        let _ = writeln!(out, "/// {} : {}.", e.name, e.doc);
        let _ = writeln!(out, "pub const EVENT_{}: i32 = {};", e.constant, e.id);
    }
    out.push_str("\n/// Les numéros des appels de machines (`on_machines`).\n");
    for m in &catalog.machine_calls {
        let _ = writeln!(out, "/// {} : {}.", m.name, m.doc);
        let _ = writeln!(out, "pub const MACHINES_{}: i32 = {};", m.constant, m.id);
    }
}

fn signature(f: &Function, named: bool) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|(name, ty)| {
            let name = if named { name.as_str() } else { "_" };
            format!("{name}: {}", ty.rust())
        })
        .collect();
    let ret = f.ret.map_or(String::new(), |t| format!(" -> {}", t.rust()));
    format!("fn {}({}){ret}", f.name, params.join(", "))
}

/// Les imports du mod : déclarés pour WebAssembly, inertes ailleurs.
fn sdk_ffi(catalog: &Catalog) -> String {
    let mut out = String::from(HEADER);
    out.push_str("//! Les fonctions que le jeu fournit au mod (module d'import `");
    out.push_str(&catalog.module);
    out.push_str("`), sans enveloppe : `host.rs` les rend sûres.\n\n#![allow(dead_code)]\n\n");
    constants(catalog, &mut out);
    let _ = writeln!(
        out,
        "\n#[cfg(target_arch = \"wasm32\")]\n#[link(wasm_import_module = \"{}\")]\nunsafe extern \"C\" {{",
        catalog.module
    );
    for f in &catalog.functions {
        let _ = writeln!(out, "    /// {}\n    pub {};", f.doc, signature(f, true));
    }
    out.push_str("}\n\n");
    out.push_str("/// Hors WebAssembly (compilation native du workspace, tests des mods) : des\n");
    out.push_str("/// fonctions inertes, mêmes signatures.\n");
    out.push_str("#[cfg(not(target_arch = \"wasm32\"))]\nmod inert {\n");
    for f in &catalog.functions {
        let body = f.fallback.as_deref().unwrap_or("");
        let _ = writeln!(
            out,
            "    #[allow(clippy::missing_safety_doc, clippy::too_many_arguments)]\n    pub unsafe {} {{ {body} }}",
            signature(f, false)
        );
    }
    out.push_str("}\n\n#[cfg(not(target_arch = \"wasm32\"))]\npub use inert::*;\n");
    out
}

/// Les numéros, côté jeu.
fn host_abi(catalog: &Catalog) -> String {
    let mut out = String::from(HEADER);
    out.push_str("//! Les numéros de l'interface des mods, côté jeu.\n\n");
    constants(catalog, &mut out);
    out
}

/// Remplace dans `doc` le texte entre `<!-- hooks:<name> -->` et `<!-- /hooks:<name> -->`.
fn replace_section(doc: &str, name: &str, content: &str) -> Result<String> {
    let start = format!("<!-- hooks:{name} -->");
    let end = format!("<!-- /hooks:{name} -->");
    let (Some(a), Some(b)) = (doc.find(&start), doc.find(&end)) else {
        bail!("{DOC} : marqueurs {start} … {end} introuvables");
    };
    if b < a {
        bail!("{DOC} : {end} avant {start}");
    }
    Ok(format!("{}{start}\n{content}{}", &doc[..a], &doc[b..]))
}

fn doc(current: &str, catalog: &Catalog) -> Result<String> {
    let mut events =
        String::from("| Événement | Quand | Données (interface binaire) |\n|---|---|---|\n");
    for e in &catalog.events {
        let _ = writeln!(
            events,
            "| `{}` ({}) | {} | {} |",
            e.name, e.id, e.doc, e.data
        );
    }
    let mut machines = String::from("| `MachineCall` | Quand |\n|---|---|\n");
    for m in &catalog.machine_calls {
        let _ = writeln!(machines, "| `{}` ({}) | {} |", m.name, m.id, m.doc);
    }
    let mut imports = String::new();
    let mut groups: Vec<&str> = Vec::new();
    for f in &catalog.functions {
        if !groups.contains(&f.group.as_str()) {
            groups.push(&f.group);
        }
    }
    for group in groups {
        let _ = writeln!(imports, "*{group}*\n\n| Fonction | Rôle |\n|---|---|");
        for f in catalog.functions.iter().filter(|f| f.group == group) {
            let _ = writeln!(
                imports,
                "| `{}` | {} |",
                signature(f, true).trim_start_matches("fn "),
                f.doc
            );
        }
        imports.push('\n');
    }
    let doc = replace_section(current, "events", &events)?;
    let doc = replace_section(&doc, "machines", &machines)?;
    replace_section(&doc, "imports", &imports)
}

/// Le code mis en forme comme `cargo fmt` le ferait.
fn rustfmt(code: &str) -> Result<String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("rustfmt introuvable")?;
    child
        .stdin
        .take()
        .context("pas d'entrée")?
        .write_all(code.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!("rustfmt refuse le code généré :\n{code}");
    }
    Ok(String::from_utf8(output.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_is_valid() {
        let text = std::fs::read_to_string(crate::root().join(CATALOG)).unwrap();
        let catalog = Catalog::parse(&text).unwrap();
        assert!(catalog.functions.iter().any(|f| f.name == "log"));
    }

    #[test]
    fn a_return_value_needs_a_fallback() {
        let text = r#"#![enable(implicit_some)] (module: "voxel", functions: [(name: "f", group: "g", params: [], ret: I32, doc: "")], events: [], machine_calls: [])"#;
        let error = Catalog::parse(text).unwrap_err().to_string();
        assert!(error.contains("fallback"), "{error}");
    }

    #[test]
    fn sections_are_replaced_between_markers() {
        let doc = "avant\n<!-- hooks:x -->\nancien\n<!-- /hooks:x -->\naprès\n";
        let new = replace_section(doc, "x", "nouveau\n").unwrap();
        assert_eq!(
            new,
            "avant\n<!-- hooks:x -->\nnouveau\n<!-- /hooks:x -->\naprès\n"
        );
        assert!(replace_section(doc, "y", "").is_err());
    }

    #[test]
    fn generated_rust_has_both_targets() {
        let text = std::fs::read_to_string(crate::root().join(CATALOG)).unwrap();
        let ffi = sdk_ffi(&Catalog::parse(&text).unwrap());
        assert!(ffi.contains("pub fn get_block(g: i32, x: i32, y: i32, z: i32) -> i32;"));
        assert!(ffi.contains("pub unsafe fn block_id(_: i32, _: i32) -> i32 { -1 }"));
        assert!(ffi.contains("pub const EVENT_BLOCK_BROKEN: i32 = 1;"));
    }
}
