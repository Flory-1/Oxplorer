use super::*;

/// Each node as `depth key=value` (containers: `depth key{children}`).
fn outline(t: &Tree) -> Vec<String> {
    t.nodes
        .iter()
        .map(|n| {
            let key = n.key.as_deref().unwrap_or("-");
            match n.is_container() && n.value.is_empty() {
                true => format!("{} {key}{{{}}}", n.depth, n.children),
                false => format!("{} {key}={}", n.depth, n.value),
            }
        })
        .collect()
}

#[test]
fn formats_by_extension() {
    assert_eq!(Format::of("json"), Some(Format::Json));
    assert_eq!(Format::of("csproj"), Some(Format::Xml));
    assert_eq!(Format::of("yml"), Some(Format::Yaml));
    assert_eq!(Format::of("toml"), Some(Format::Toml));
    assert_eq!(Format::of("svg"), None);
    assert_eq!(Format::of("txt"), None);
}

#[test]
fn json_keeps_the_order_and_types() {
    let t = parse(
        Format::Json,
        r#"{"z": 1, "a": [true, null, 2.5, "x"], "m": {}, "n": -3}"#,
    )
    .unwrap();
    assert_eq!(
        outline(&t),
        [
            "0 -{4}", "1 z=1", "1 a{4}", "2 -=true", "2 -=null", "2 -=2.5", "2 -=x", "1 m{0}",
            "1 n=-3"
        ]
    );
    let kinds: Vec<Kind> = t.nodes.iter().map(|n| n.kind).collect();
    assert_eq!(
        kinds,
        [
            Kind::Object,
            Kind::Number,
            Kind::Array,
            Kind::Bool,
            Kind::Null,
            Kind::Number,
            Kind::String,
            Kind::Object,
            Kind::Number
        ]
    );
    assert_eq!(t.nodes[2].end, 7);
    assert_eq!(t.children(0).collect::<Vec<_>>(), [1, 2, 7, 8]);
    assert!(!t.truncated);
}

#[test]
fn json_with_comments_and_trailing_commas() {
    let text =
        "{\n  // a comment\n  \"a\": \"x // not a comment\", /* block\n */ \"b\": [1, 2,],\n}";
    let t = parse(Format::Json, text).unwrap();
    assert_eq!(
        outline(&t),
        [
            "0 -{2}",
            "1 a=x // not a comment",
            "1 b{2}",
            "2 -=1",
            "2 -=2"
        ]
    );
    assert_eq!(strip_len(text), text.len());
}

fn strip_len(text: &str) -> usize {
    formats::strip_jsonc(text).len()
}

#[test]
fn json_errors_have_a_position() {
    let e = parse(Format::Json, "{\n  \"a\": 1,\n  \"b\" 2\n}").unwrap_err();
    assert_eq!((e.line, e.column), (3, 7));
    assert!(!e.message.contains("line"), "{}", e.message);
    assert!(parse(Format::Json, "[1] x").is_err());
}

#[test]
fn big_documents_are_truncated() {
    let text = format!("[{}]", vec!["1"; MAX_NODES + 10].join(","));
    let t = parse(Format::Json, &text).unwrap();
    assert!(t.truncated);
    assert_eq!(t.nodes.len(), MAX_NODES);
    assert_eq!(t.nodes[0].end as usize, MAX_NODES);
}

#[test]
fn xml_elements_attributes_and_text() {
    let t = parse(
        Format::Xml,
        r#"<?xml version="1.0"?>
<!-- comment -->
<root lang="en">
  <item id="1">Tea &amp; cake</item>
  <item id="2"/>
  <name>Ann &#x41;</name>
  <empty></empty>
  <![CDATA[raw <text>]]>
</root>"#,
    )
    .unwrap();
    assert_eq!(
        outline(&t),
        [
            "0 root{6}",
            "1 lang=en",
            "1 item{2}",
            "2 id=1",
            "2 -=Tea & cake",
            "1 item{1}",
            "2 id=2",
            "1 name=Ann A",
            "1 empty{0}",
            "1 -=raw <text>",
        ]
    );
    assert_eq!(t.path(4), "/root/item[1]/text()");
    assert_eq!(t.path(6), "/root/item[2]/@id");
    assert_eq!(t.path(7), "/root/name");
    assert_eq!(t.path(0), "/root");
}

#[test]
fn xml_errors_have_a_position() {
    let e = parse(Format::Xml, "<a>\n  <b>\n</a>").unwrap_err();
    assert_eq!(e.line, 3, "{e:?}");
    let e = parse(Format::Xml, "<a>\n<b></b>").unwrap_err();
    assert_eq!(e.line, 2, "{e:?}");
}

#[test]
fn yaml_documents() {
    let t = parse(
        Format::Yaml,
        "name: app\nversion: 1.2\nports:\n  - 80\n  - 443\ndebug: false\nnone: ~\n",
    )
    .unwrap();
    assert_eq!(
        outline(&t),
        [
            "0 -{5}",
            "1 name=app",
            "1 version=1.2",
            "1 ports{2}",
            "2 -=80",
            "2 -=443",
            "1 debug=false",
            "1 none=null"
        ]
    );
    let t = parse(Format::Yaml, "a: 1\n---\nb: 2\n").unwrap();
    assert_eq!(
        outline(&t),
        ["0 -{2}", "1 -{1}", "2 a=1", "1 -{1}", "2 b=2"]
    );
    let e = parse(Format::Yaml, "a: [1, 2\nb: 3\n").unwrap_err();
    assert!(e.line >= 1 && !e.message.is_empty(), "{e:?}");
    assert!(parse(Format::Yaml, "").unwrap().nodes.is_empty());
}

#[test]
fn toml_in_file_order() {
    let t = parse(
        Format::Toml,
        "title = \"x\"\nwhen = 1979-05-27\n[server]\nport = 8080\n[[bin]]\nname = \"a\"\n",
    )
    .unwrap();
    assert_eq!(
        outline(&t),
        [
            "0 -{4}",
            "1 title=x",
            "1 when=1979-05-27",
            "1 server{1}",
            "2 port=8080",
            "1 bin{1}",
            "2 -{1}",
            "3 name=a"
        ]
    );
    assert_eq!(t.nodes[2].kind, Kind::Date);
    let e = parse(Format::Toml, "a = 1\nb = \n").unwrap_err();
    assert_eq!(e.line, 2, "{e:?}");
}

#[test]
fn paths_and_copies() {
    let t = parse(Format::Json, r#"{"a/b": [{"~x": 1}], "s": "text"}"#).unwrap();
    assert_eq!(t.path(0), "");
    assert_eq!(t.path(1), "/a~1b");
    assert_eq!(t.path(3), "/a~1b/0/~0x");
    assert_eq!(t.copy_text(4), "text");
    assert_eq!(t.copy_text(1), "[\n  {\n    \"~x\": 1\n  }\n]");
    assert_eq!(t.copy_text(0).lines().count(), 8);
}

#[test]
fn collapsing_hides_descendants() {
    let t = parse(Format::Json, r#"{"a": {"b": {"c": 1}}, "d": 2}"#).unwrap();
    let open = t.default_open();
    assert_eq!(open, [true, true, false, false, true]);
    assert_eq!(t.visible(&open), [0, 1, 2, 4]);
    assert_eq!(t.visible(&[false; 5]), [0]);
    assert_eq!(t.visible(&[true; 5]), [0, 1, 2, 3, 4]);
}

#[test]
fn line_columns() {
    assert_eq!(line_col("ab\ncd", 4), (2, 2));
    assert_eq!(line_col("äb", 2), (1, 2));
    assert_eq!(line_col("x", 99), (1, 2));
}
