//! Le journal : la console, et une copie dans `logs/` (voir `docs/plan-installation.md`).
//!
//! Chaque lancement écrit `logs/latest.log` (`server-latest.log` pour le serveur dédié, pour qu'un
//! jeu et un serveur lancés du même dossier ne s'écrasent pas). Le journal du lancement précédent
//! est d'abord renommé d'après sa date (`voxel-2026-11-22_14.03.12.log`) ; on garde les
//! [`KEPT`] derniers de chaque programme. Les dates sont en temps universel (UTC).

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Le nombre de journaux anciens gardés par programme.
pub const KEPT: usize = 10;

/// Le programme qui écrit le journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Program {
    /// `voxel`, le jeu (et son serveur intégré).
    Game,
    /// `voxel-server`, le serveur dédié.
    Server,
}

impl Program {
    /// Le journal du lancement en cours.
    fn latest(self) -> &'static str {
        match self {
            Self::Game => "latest.log",
            Self::Server => "server-latest.log",
        }
    }

    /// Le début du nom des journaux anciens.
    fn prefix(self) -> &'static str {
        match self {
            Self::Game => "voxel-",
            Self::Server => "server-",
        }
    }

    fn default_filter(self) -> &'static str {
        match self {
            Self::Game => "warn,voxel=info,voxel_sim::logs=info",
            Self::Server => "warn,voxel_sim=info",
        }
    }
}

/// Une date lisible et triable, en temps universel : `2026-11-22_14.03.12`. Sert aussi aux noms
/// des captures d'écran.
pub fn stamp(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Jours depuis 1970 → date civile (algorithme de Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}.{:02}.{:02}",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// Range le journal précédent sous sa date et ne garde que les [`KEPT`] derniers ; rend le fichier
/// du lancement en cours, ouvert.
fn rotate(dir: &Path, program: Program) -> io::Result<(PathBuf, File)> {
    fs::create_dir_all(dir)?;
    let latest = dir.join(program.latest());
    if let Ok(meta) = fs::metadata(&latest) {
        let when = meta.modified().unwrap_or_else(|_| SystemTime::now());
        let mut archive = dir.join(format!("{}{}.log", program.prefix(), stamp(when)));
        // Deux lancements dans la même seconde : un suffixe.
        let mut n = 1;
        while archive.exists() {
            archive = dir.join(format!("{}{}-{n}.log", program.prefix(), stamp(when)));
            n += 1;
        }
        fs::rename(&latest, &archive)?;
    }
    let mut old: Vec<PathBuf> = fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with(program.prefix()) && n.ends_with(".log") && n != program.latest()
            })
        })
        .collect();
    old.sort();
    for path in old.iter().take(old.len().saturating_sub(KEPT)) {
        let _ = fs::remove_file(path);
    }
    let file = File::create(&latest)?;
    Ok((latest, file))
}

/// La console, plus le fichier s'il a pu s'ouvrir.
struct Tee(Option<File>);

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(buf);
        if let Some(file) = &mut self.0
            && file.write_all(buf).is_err()
        {
            self.0 = None;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &mut self.0 {
            let _ = file.flush();
        }
        io::stderr().flush()
    }
}

/// Démarre le journal (après [`crate::paths::init`]) : `RUST_LOG` choisit les niveaux, comme avant.
/// Dit où sont les dossiers du jeu et où s'écrit le journal.
pub fn init(program: Program) {
    let paths = crate::paths::get();
    let opened = rotate(&paths.logs, program);
    let (written, file) = match opened {
        Ok((path, file)) => (Ok(path), Some(file)),
        Err(error) => (Err(error), None),
    };
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(program.default_filter()),
    )
    .target(env_logger::Target::Pipe(Box::new(Tee(file))))
    .init();
    log::info!(
        "dossiers ({:?}) : racine {}, jeu de base {}",
        paths.layout,
        paths.root.display(),
        paths.game.display()
    );
    match written {
        Ok(path) => log::info!("journal : {}", path.display()),
        Err(error) => log::warn!(
            "journal : {} inaccessible ({error}), la console seulement",
            paths.logs.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stamps_are_civil_dates_in_utc() {
        assert_eq!(stamp(UNIX_EPOCH), "1970-01-01_00.00.00");
        // 22 novembre 2026, 14:03:12 UTC.
        let t = UNIX_EPOCH + Duration::from_secs(1_795_356_192);
        assert_eq!(stamp(t), "2026-11-22_14.03.12");
        // Un 29 février.
        let leap = UNIX_EPOCH + Duration::from_secs(1_709_164_800);
        assert_eq!(stamp(leap), "2024-02-29_00.00.00");
    }

    #[test]
    fn the_previous_log_is_archived_and_old_ones_pruned() {
        let dir = std::env::temp_dir().join(format!("voxel-logs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for i in 0..KEPT + 3 {
            fs::write(
                dir.join(format!("voxel-2000-01-{:02}_00.00.00.log", i + 1)),
                "",
            )
            .unwrap();
        }
        // Le journal d'un serveur lancé du même dossier n'est pas touché.
        fs::write(dir.join("server-latest.log"), "serveur").unwrap();
        fs::write(dir.join("latest.log"), "avant").unwrap();

        let (latest, mut file) = rotate(&dir, Program::Game).unwrap();
        file.write_all(b"maintenant").unwrap();
        assert_eq!(fs::read_to_string(&latest).unwrap(), "maintenant");
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        let archived = names.iter().filter(|n| n.starts_with("voxel-")).count();
        assert_eq!(archived, KEPT);
        // Le plus récent (le `latest.log` d'avant) est gardé, les plus vieux partent.
        assert!(!names.contains(&"voxel-2000-01-01_00.00.00.log".to_string()));
        let previous = names
            .iter()
            .filter(|n| n.starts_with("voxel-") && !n.starts_with("voxel-2000"))
            .count();
        assert_eq!(previous, 1);
        assert_eq!(
            fs::read_to_string(dir.join("server-latest.log")).unwrap(),
            "serveur"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
