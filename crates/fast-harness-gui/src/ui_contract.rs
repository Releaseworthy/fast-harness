//! The UI contract (PLAN.md D51, `docs/ui-contract.md`): every property,
//! callback and function of `MainWindow` and of the exported globals that the
//! Rust side uses. `fast-harness-gui-live` checks a user's `.slint` files
//! against it before it loads them; a test regenerates it from the Rust and
//! `.slint` sources so the document cannot drift.

/// The document, embedded: the live variant reads its list at run time.
pub const DOC: &str = include_str!("../../../docs/ui-contract.md");

/// What a contract entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Property,
    Callback,
    Function,
}

/// One line of the contract: `MainWindow.error-text`, a property of type `string`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// `MainWindow` or a global's name.
    pub owner: String,
    /// The `.slint` name (kebab case).
    pub name: String,
    pub kind: Kind,
    /// The `.slint` type of a property (`string`, `[MsgRow]`), empty otherwise.
    pub ty: String,
}

impl Entry {
    /// The document line of this entry.
    #[cfg(test)]
    pub fn line(&self) -> String {
        let what = match self.kind {
            Kind::Property => format!("property <{}>", self.ty),
            Kind::Callback => "callback".into(),
            Kind::Function => "function".into(),
        };
        format!("- `{}.{}`: {what}", self.owner, self.name)
    }
}

/// The entries of the contract document (its `- ` list lines).
pub fn parse(doc: &str) -> Vec<Entry> {
    doc.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Entry> {
    let rest = line.strip_prefix("- `")?;
    let (path, what) = rest.split_once("`: ")?;
    let (owner, name) = path.split_once('.')?;
    let (kind, ty) = if let Some(ty) = what.strip_prefix("property <") {
        (Kind::Property, ty.strip_suffix('>')?.to_string())
    } else if what == "callback" {
        (Kind::Callback, String::new())
    } else if what == "function" {
        (Kind::Function, String::new())
    } else {
        return None;
    };
    Some(Entry {
        owner: owner.into(),
        name: name.into(),
        kind,
        ty,
    })
}

#[cfg(test)]
#[path = "ui_contract_scan.rs"]
mod scan;
