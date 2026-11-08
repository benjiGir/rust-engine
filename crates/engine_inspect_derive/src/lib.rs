//! `#[derive(Inspect)]` : génère l'implémentation du trait `Inspect` pour une struct.
//!
//! # Une macro procédurale
//!
//! `macro_rules!` remplace des motifs de texte. Une macro procédurale, elle, est une
//! **fonction Rust exécutée par le compilateur** : elle reçoit le code de la struct sous
//! forme de jetons (`TokenStream`), le transforme en arbre lisible avec `syn`, puis
//! écrit le nouveau code avec `quote!`, où `#nom` insère une valeur.
//!
//! Pour la struct :
//! ```ignore
//! #[derive(Inspect)]
//! struct Health { current: f32, max: f32 }
//! ```
//! elle produit à peu près :
//! ```ignore
//! impl Inspect for Health {
//!     fn inspect(&mut self, ui: &mut egui::Ui, label: &str) -> bool {
//!         let mut changed = false;
//!         ui.collapsing(label, |ui| {
//!             changed |= self.current.inspect(ui, "current");
//!             changed |= self.max.inspect(ui, "max");
//!         });
//!         changed
//!     }
//!     fn field_names() -> &'static [&'static str] { &["current", "max"] }
//! }
//! ```
//! Un champ marqué `#[inspect(skip)]` n'apparaît pas.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

#[proc_macro_derive(Inspect, attributes(inspect))]
pub fn derive_inspect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    // Les génériques de la struct (`struct Foo<T>`) sont recopiés tels quels.
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();

    let Data::Struct(data) = &input.data else {
        // `compile_error!` : une erreur claire, à l'endroit du derive, plutôt qu'un plantage.
        return syn::Error::new_spanned(name, "Inspect ne s'applique qu'aux structs")
            .to_compile_error()
            .into();
    };

    // Les champs à afficher : (accès au champ, nom affiché).
    let mut accesses = Vec::new();
    let mut labels = Vec::new();
    match &data.fields {
        Fields::Named(fields) => {
            for field in &fields.named {
                if is_skipped(field) {
                    continue;
                }
                let ident = field.ident.as_ref().expect("champ nommé");
                accesses.push(quote! { self.#ident });
                labels.push(ident.to_string());
            }
        }
        Fields::Unnamed(fields) => {
            for (i, field) in fields.unnamed.iter().enumerate() {
                if is_skipped(field) {
                    continue;
                }
                // `self.0`, `self.1`... : un index de tuple, pas un nombre ordinaire.
                let index = syn::Index::from(i);
                accesses.push(quote! { self.#index });
                labels.push(i.to_string());
            }
        }
        Fields::Unit => {}
    }

    let expanded = quote! {
        impl #impl_generics ::engine_inspect::Inspect for #name #type_generics #where_clause {
            fn inspect(&mut self, ui: &mut ::engine_inspect::egui::Ui, label: &str) -> bool {
                let mut changed = false;
                ui.collapsing(label, |ui| {
                    // `#( ... )*` répète le bloc pour chaque élément des listes.
                    #( changed |= ::engine_inspect::Inspect::inspect(&mut #accesses, ui, #labels); )*
                });
                changed
            }

            fn field_names() -> &'static [&'static str] {
                &[#(#labels),*]
            }
        }
    };
    expanded.into()
}

/// Vrai si le champ porte `#[inspect(skip)]`.
fn is_skipped(field: &syn::Field) -> bool {
    field.attrs.iter().any(|attr| {
        attr.path().is_ident("inspect")
            && attr
                .parse_args::<syn::Ident>()
                .is_ok_and(|ident| ident == "skip")
    })
}
