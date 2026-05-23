//! Normalized crate-doc schema, decoupled from any specific
//! `rustdoc-types` version.
//!
//! docs.rs serves rustdoc-JSON at multiple schema versions depending on
//! when each crate was last built. The repository deserializes raw
//! bytes into the matching upstream type ([`rustdoc_types::Crate`] for
//! format 57, [`rustdoc_types_56::Crate`] for format 56), then
//! [`From`]-converts into the types defined here so the use case never
//! references either upstream crate directly.
//!
//! Only the fields the use case actually reads are modeled. Unknown
//! item kinds (kinds we don't render or future additions) collapse to
//! [`DocsRsItemKind::Other`] and are dropped by the search walk.

use std::collections::HashMap;

/// Subset of [`rustdoc_types::Crate`] the use case reads. Format-version
/// agnostic; both 0.56 and 0.57 JSONs are normalized into this shape.
#[derive(Debug, Clone, Default)]
pub struct DocsRsCrate {
    /// All items keyed by their numeric id. Mirrors
    /// [`rustdoc_types::Crate::index`] but with the [`rustdoc_types::Id`]
    /// newtype unwrapped to a plain `u32`.
    pub index: HashMap<u32, DocsRsItem>,
    /// Per-id summaries that include the addressable path and kind.
    /// Mirrors [`rustdoc_types::Crate::paths`].
    pub paths: HashMap<u32, DocsRsItemSummary>,
}

/// Subset of [`rustdoc_types::Item`] the use case reads — just the
/// fields needed for doc-comment search and result ranking.
#[derive(Debug, Clone)]
pub struct DocsRsItem {
    /// Item's short name (e.g. `"Deserialize"`). `None` for items
    /// without names (impls).
    pub name: Option<String>,
    /// Markdown doc body. `None` if undocumented, `Some("")` if doc
    /// is empty.
    pub docs: Option<String>,
}

/// Subset of [`rustdoc_types::ItemSummary`] the use case reads.
#[derive(Debug, Clone)]
pub struct DocsRsItemSummary {
    /// `0` for the local crate; non-zero for items pulled in from a
    /// foreign crate (re-exports, intra-doc references). The search
    /// walk filters non-zero so result URLs always resolve under the
    /// requested crate's docs root.
    pub crate_id: u32,
    /// Fully-qualified path components, e.g. `["serde", "de", "value",
    /// "U8Deserializer"]`. `path[0]` is the crate lib name; the use
    /// case strips it when assembling the URL relative to the crate
    /// docs root.
    pub path: Vec<String>,
    /// Item kind, narrowed to the addressable variants. Anything not
    /// in [`DocsRsItemKind`]'s explicit list (impls, fields, variants,
    /// future additions) maps to [`DocsRsItemKind::Other`].
    pub kind: DocsRsItemKind,
}

/// Addressable item kinds — anything rustdoc renders to its own
/// `{kind}.{name}.html` page. Everything else (impls, fields, variants,
/// associated items, extern crates/types, use statements, future
/// additions) collapses to [`Self::Other`] and is filtered out by the
/// search walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocsRsItemKind {
    /// `mod foo`. Renders to `{path}/index.html`.
    Module,
    /// `struct Foo`.
    Struct,
    /// `union Foo`.
    Union,
    /// `enum Foo`.
    Enum,
    /// `fn foo()`. Top-level functions only; methods are not addressable.
    Function,
    /// `type Foo = ...`.
    TypeAlias,
    /// `const FOO: ...`.
    Constant,
    /// `trait Foo`.
    Trait,
    /// `trait Foo = ...`.
    TraitAlias,
    /// `static FOO: ...`.
    Static,
    /// `macro_rules!` and proc-macro function-style macros.
    Macro,
    /// `#[proc_macro_derive]` macros.
    ProcDerive,
    /// `#[proc_macro_attribute]` macros.
    ProcAttribute,
    /// Core's built-in attribute documentation (e.g. `#[no_mangle]`).
    /// Uses the same `attr.{name}.html` URL shape as [`Self::ProcAttribute`].
    Attribute,
    /// Built-in primitive types (only from `core`/`std`).
    Primitive,
    /// `keyword.{name}.html` doc pages (only from `core`/`std`).
    Keyword,
    /// Anything we don't model — impls, fields, variants, associated
    /// items, extern crates/types, use statements, future rustdoc
    /// additions. Skipped by the search walk.
    Other,
}

// Generate `From<rustdoc_types[_NN]::Crate> for DocsRsCrate` for each
// supported upstream crate. The two crates have byte-identical schemas
// for the fields we care about (the 56→57 schema bump only added a
// required `path` field to `ExternalCrate`, which we don't touch), so
// the conversion bodies are line-for-line identical aside from the
// crate path. Macro keeps that fact load-bearing — adding a new
// version is `impl_from_upstream!(rustdoc_types_NN);` plus a dispatch
// arm in the repository.
macro_rules! impl_from_upstream {
    ($upstream:ident) => {
        impl From<$upstream::Crate> for DocsRsCrate {
            fn from(c: $upstream::Crate) -> Self {
                let index = c
                    .index
                    .into_iter()
                    .map(|(id, item)| {
                        (
                            id.0,
                            DocsRsItem {
                                name: item.name,
                                docs: item.docs,
                            },
                        )
                    })
                    .collect();
                let paths = c
                    .paths
                    .into_iter()
                    .map(|(id, summary)| {
                        (
                            id.0,
                            DocsRsItemSummary {
                                crate_id: summary.crate_id,
                                path: summary.path,
                                kind: summary.kind.into(),
                            },
                        )
                    })
                    .collect();
                DocsRsCrate { index, paths }
            }
        }

        impl From<$upstream::ItemKind> for DocsRsItemKind {
            fn from(k: $upstream::ItemKind) -> Self {
                use $upstream::ItemKind as K;
                match k {
                    K::Module => Self::Module,
                    K::Struct => Self::Struct,
                    K::Union => Self::Union,
                    K::Enum => Self::Enum,
                    K::Function => Self::Function,
                    K::TypeAlias => Self::TypeAlias,
                    K::Constant => Self::Constant,
                    K::Trait => Self::Trait,
                    K::TraitAlias => Self::TraitAlias,
                    K::Static => Self::Static,
                    K::Macro => Self::Macro,
                    K::ProcDerive => Self::ProcDerive,
                    K::ProcAttribute => Self::ProcAttribute,
                    K::Attribute => Self::Attribute,
                    K::Primitive => Self::Primitive,
                    K::Keyword => Self::Keyword,
                    // Everything else (Impl / StructField / Variant /
                    // AssocConst / AssocType / ExternCrate / Use /
                    // ExternType, plus any future kinds) — the search
                    // walk drops these because they don't have a
                    // dedicated rustdoc page.
                    _ => Self::Other,
                }
            }
        }
    };
}

impl_from_upstream!(rustdoc_types);
impl_from_upstream!(rustdoc_types_56);
