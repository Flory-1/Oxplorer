//! Data files as a tree for the preview's Tree tab: JSON (comments
//! allowed), XML, YAML and TOML are parsed into one kind of node.
//!
//! The nodes are stored flat in document order: a node's descendants
//! follow it, up to its `end`. Collapsing a node skips to its `end`.

mod formats;

use std::fmt;

/// Larger files aren't parsed (the Text tab still shows their beginning).
pub const MAX_BYTES: u64 = 16 * 1024 * 1024;
/// More nodes than this aren't kept (the tree is marked as truncated).
pub const MAX_NODES: usize = 200_000;
/// Deeper nesting is cut off.
const MAX_DEPTH: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Json,
    Xml,
    Yaml,
    Toml,
}

impl Format {
    /// The format of a file with extension `ext` (lower case).
    pub fn of(ext: &str) -> Option<Format> {
        Some(match ext {
            "json" | "jsonc" | "geojson" | "topojson" | "webmanifest" | "har" | "ipynb" => {
                Format::Json
            }
            "xml" | "csproj" | "vbproj" | "fsproj" | "vcxproj" | "props" | "targets" | "nuspec"
            | "resx" | "xaml" | "config" | "xsd" | "xsl" | "xslt" | "wsdl" | "rss" | "atom"
            | "gpx" | "kml" | "plist" | "manifest" => Format::Xml,
            "yaml" | "yml" => Format::Yaml,
            "toml" => Format::Toml,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Json => "JSON",
            Format::Xml => "XML",
            Format::Yaml => "YAML",
            Format::Toml => "TOML",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Object,
    Array,
    /// An XML element; its attributes and contents are its children.
    Element,
    Attribute,
    /// Text inside an XML element.
    Text,
    String,
    Number,
    Bool,
    Null,
    /// A TOML date or time.
    Date,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: Kind,
    /// Key in its object, or the name of an element or attribute. `None`
    /// for array items and text.
    pub key: Option<String>,
    /// A scalar's value; an element's text when that is all it contains.
    pub value: String,
    pub depth: u32,
    pub parent: Option<u32>,
    /// Position among the parent's children.
    pub index: u32,
    /// Number of direct children.
    pub children: u32,
    /// Index after the node's last descendant.
    pub end: u32,
}

impl Node {
    /// Objects, arrays and elements (even empty ones).
    pub fn is_container(&self) -> bool {
        matches!(self.kind, Kind::Object | Kind::Array | Kind::Element)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tree {
    pub format: Format,
    pub nodes: Vec<Node>,
    /// Stopped at [`MAX_NODES`] or [`MAX_DEPTH`].
    pub truncated: bool,
}

/// Why a file couldn't be parsed, and where (1-based; 0 if unknown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl ParseError {
    /// An error at byte `offset` of `text`.
    pub(crate) fn at(message: impl Into<String>, text: &str, offset: usize) -> Self {
        let (line, column) = line_col(text, offset);
        Self {
            message: message.into(),
            line,
            column,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Line and column (1-based, in characters) of byte `offset` in `text`.
pub fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |n| n + 1);
    (line, before[start..].chars().count() + 1)
}

/// Parse `text` as `format`.
pub fn parse(format: Format, text: &str) -> Result<Tree, ParseError> {
    let mut b = Builder::default();
    match format {
        Format::Json => formats::json(&mut b, text)?,
        Format::Xml => formats::xml(&mut b, text)?,
        Format::Yaml => formats::yaml(&mut b, text)?,
        Format::Toml => formats::toml(&mut b, text)?,
    }
    Ok(b.finish(format))
}

/// Builds the flat node list while a parser walks the document.
#[derive(Default)]
pub(crate) struct Builder {
    nodes: Vec<Node>,
    /// The containers currently open.
    stack: Vec<u32>,
    truncated: bool,
}

impl Builder {
    /// Whether no more nodes are taken (the tree is then truncated).
    pub(crate) fn full(&mut self) -> bool {
        let full = self.nodes.len() >= MAX_NODES || self.stack.len() as u32 >= MAX_DEPTH;
        self.truncated |= full;
        full
    }

    fn push(&mut self, kind: Kind, key: Option<String>, value: String) -> Option<u32> {
        if self.full() {
            return None;
        }
        let at = self.nodes.len() as u32;
        let parent = self.stack.last().copied();
        let index = parent.map_or(0, |p| {
            let p = &mut self.nodes[p as usize];
            p.children += 1;
            p.children - 1
        });
        self.nodes.push(Node {
            kind,
            key,
            value,
            depth: self.stack.len() as u32,
            parent,
            index,
            children: 0,
            end: at + 1,
        });
        Some(at)
    }

    /// Add a scalar. Returns false when full.
    pub(crate) fn leaf(&mut self, kind: Kind, key: Option<String>, value: String) -> bool {
        self.push(kind, key, value).is_some()
    }

    /// Start a container; its children follow until [`Builder::close`].
    /// Returns false when full (nothing was added; don't close it).
    pub(crate) fn open(&mut self, kind: Kind, key: Option<String>) -> bool {
        match self.push(kind, key, String::new()) {
            Some(at) => {
                self.stack.push(at);
                true
            }
            None => false,
        }
    }

    /// End the innermost container. An element holding nothing but text
    /// shows that text as its value.
    pub(crate) fn close(&mut self) {
        let Some(at) = self.stack.pop() else {
            return;
        };
        let len = self.nodes.len() as u32;
        let node = &self.nodes[at as usize];
        if node.kind == Kind::Element
            && node.children == 1
            && len == at + 2
            && self.nodes[at as usize + 1].kind == Kind::Text
        {
            let text = self.nodes.pop().expect("checked").value;
            let node = &mut self.nodes[at as usize];
            node.value = text;
            node.children = 0;
            node.end = at + 1;
            return;
        }
        self.nodes[at as usize].end = len;
    }

    /// How many containers are open.
    pub(crate) fn open_count(&self) -> usize {
        self.stack.len()
    }

    /// The name of the innermost open container.
    pub(crate) fn innermost(&self) -> Option<&str> {
        let at = *self.stack.last()?;
        self.nodes[at as usize].key.as_deref()
    }

    pub(crate) fn truncated(&self) -> bool {
        self.truncated
    }

    fn finish(mut self, format: Format) -> Tree {
        while !self.stack.is_empty() {
            self.close();
        }
        Tree {
            format,
            nodes: self.nodes,
            truncated: self.truncated,
        }
    }
}

impl Tree {
    /// Which nodes are open at first: the top two levels.
    pub fn default_open(&self) -> Vec<bool> {
        self.nodes.iter().map(|n| n.depth < 2).collect()
    }

    /// The rows shown when only the nodes with `open[i]` show their children.
    pub fn visible(&self, open: &[bool]) -> Vec<u32> {
        let mut rows = Vec::new();
        let mut i = 0;
        while i < self.nodes.len() {
            rows.push(i as u32);
            i = if open.get(i).copied().unwrap_or(false) {
                i + 1
            } else {
                self.nodes[i].end as usize
            };
        }
        rows
    }

    /// The nodes from `i` up to the root, `i` first.
    fn ancestors(&self, i: u32) -> impl Iterator<Item = &Node> {
        std::iter::successors(self.nodes.get(i as usize), |n| {
            n.parent.map(|p| &self.nodes[p as usize])
        })
    }

    /// Where node `i` is: a JSON pointer (`/items/0/name`), for XML a
    /// path like `/root/item[2]/@id`.
    pub fn path(&self, i: u32) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut at = i;
        for node in self.ancestors(i) {
            match self.format {
                Format::Xml => parts.push(self.xml_step(at, node)),
                _ if node.parent.is_none() => {}
                _ => parts.push(match &node.key {
                    Some(key) => key.replace('~', "~0").replace('/', "~1"),
                    None => node.index.to_string(),
                }),
            }
            at = node.parent.unwrap_or(0);
        }
        parts.reverse();
        parts.iter().map(|p| format!("/{p}")).collect()
    }

    /// One step of an XML path: `name`, `name[2]`, `@attr` or `text()`.
    fn xml_step(&self, at: u32, node: &Node) -> String {
        let name = node.key.clone().unwrap_or_default();
        match node.kind {
            Kind::Attribute => format!("@{name}"),
            Kind::Text => "text()".to_owned(),
            _ => {
                let Some(parent) = node.parent else {
                    return name;
                };
                let same: Vec<u32> = self
                    .children(parent)
                    .filter(|&c| {
                        let c = &self.nodes[c as usize];
                        c.kind == Kind::Element && c.key == node.key
                    })
                    .collect();
                match same.iter().position(|&c| c == at) {
                    Some(n) if same.len() > 1 => format!("{name}[{}]", n + 1),
                    _ => name,
                }
            }
        }
    }

    /// The direct children of node `i`.
    pub fn children(&self, i: u32) -> impl Iterator<Item = u32> + '_ {
        let node = &self.nodes[i as usize];
        let mut next = i + 1;
        let end = node.end;
        std::iter::from_fn(move || {
            (next < end).then(|| {
                let at = next;
                next = self.nodes[at as usize].end;
                at
            })
        })
    }

    /// The text to copy for node `i`: a scalar's value, a container as
    /// JSON (XML elements: their text).
    pub fn copy_text(&self, i: u32) -> String {
        let node = &self.nodes[i as usize];
        if !node.is_container() || node.children == 0 {
            return node.value.clone();
        }
        let mut out = String::new();
        if self.format == Format::Xml {
            for n in &self.nodes[i as usize..node.end as usize] {
                if n.kind == Kind::Text || (n.kind == Kind::Element && !n.value.is_empty()) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&n.value);
                }
            }
        } else {
            self.write_json(i, 0, &mut out);
        }
        out
    }

    fn write_json(&self, i: u32, indent: usize, out: &mut String) {
        let node = &self.nodes[i as usize];
        let quote = |s: &str| serde_json::Value::String(s.to_owned()).to_string();
        match node.kind {
            Kind::Object | Kind::Array | Kind::Element => {
                let object = node.kind != Kind::Array;
                let (open, close) = if object { ('{', '}') } else { ('[', ']') };
                out.push(open);
                for (n, c) in self.children(i).enumerate() {
                    out.push_str(if n == 0 { "\n" } else { ",\n" });
                    out.push_str(&"  ".repeat(indent + 1));
                    if object {
                        let key = self.nodes[c as usize].key.as_deref().unwrap_or("");
                        out.push_str(&quote(key));
                        out.push_str(": ");
                    }
                    self.write_json(c, indent + 1, out);
                }
                if node.children > 0 {
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent));
                }
                out.push(close);
            }
            Kind::Number | Kind::Bool | Kind::Null => out.push_str(&node.value),
            _ => out.push_str(&quote(&node.value)),
        }
    }
}

#[cfg(test)]
mod tests;
