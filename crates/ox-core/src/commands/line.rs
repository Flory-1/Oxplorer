//! Command lines of custom commands: filling in the placeholders, quoting
//! for Windows command lines, and whether a line needs the shell.

use std::path::{Path, PathBuf};

/// The values the placeholders stand for.
pub struct Values<'a> {
    /// The items of this run (at least one).
    pub items: &'a [PathBuf],
    /// The current folder.
    pub folder: &'a Path,
}

/// The placeholder names, for the settings page.
pub const PLACEHOLDERS: &[&str] = &[
    "{path}",
    "{paths}",
    "{selection}",
    "{name}",
    "{stem}",
    "{ext}",
    "{parent}",
    "{folder}",
    "{n}",
];

/// cmd's built-in commands: lines starting with one run through the shell.
const BUILTINS: &[&str] = &[
    "assoc", "break", "call", "cd", "chdir", "cls", "color", "copy", "date", "del", "dir", "echo",
    "endlocal", "erase", "for", "ftype", "goto", "if", "md", "mkdir", "mklink", "move", "path",
    "pause", "popd", "prompt", "pushd", "rd", "ren", "rename", "rmdir", "set", "setlocal", "shift",
    "start", "time", "title", "type", "ver", "verify", "vol",
];

/// The folder `path` is in (the path itself for a drive root).
pub fn parent_of(path: &Path) -> PathBuf {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(path)
        .to_path_buf()
}

fn name_of(path: &Path) -> String {
    crate::entry::display_name(path)
}

/// The values of placeholder `name` (several for `{paths}`), or `None` if
/// it isn't one.
fn values(name: &str, v: &Values<'_>) -> Option<Vec<String>> {
    let first = &v.items[0];
    let text = |p: &Path| p.to_string_lossy().into_owned();
    let one = |s: String| Some(vec![s]);
    match name {
        "path" => one(text(first)),
        "paths" | "selection" => Some(v.items.iter().map(|p| text(p)).collect()),
        "name" => one(name_of(first)),
        "stem" => one(first
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| name_of(first))),
        "ext" => one(first
            .extension()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()),
        "parent" => one(text(&parent_of(first))),
        "folder" => one(text(v.folder)),
        "n" => one(v.items.len().to_string()),
        _ => None,
    }
}

/// Characters that make a value need quotes: spaces (argument separators)
/// and what cmd would read as its own syntax.
fn needs_quotes(s: &str) -> bool {
    s.is_empty()
        || s.chars()
            .any(|c| c.is_whitespace() || "\"&|<>^(),;=%!".contains(c))
}

/// `s` as one argument by the rules of `CommandLineToArgvW` (and the C
/// runtime): in quotes if needed, a quote inside as `\"`, and backslashes
/// before a quote (also the closing one) doubled.
pub fn quote(s: &str) -> String {
    if !needs_quotes(s) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    escape_into(&mut out, s, true);
    out.push('"');
    out
}

/// Append `s` for inside quotes: `"` as `\"`, backslashes before it doubled,
/// and trailing backslashes doubled if a quote follows (`quote_follows`).
fn escape_into(out: &mut String, s: &str, quote_follows: bool) {
    let mut backslashes = 0;
    for c in s.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes + 1));
                backslashes = 0;
            }
            _ => backslashes = 0,
        }
        out.push(c);
    }
    if quote_follows {
        out.extend(std::iter::repeat_n('\\', backslashes));
    }
}

/// Fill in the placeholders of `template`. Each value is quoted as one
/// argument where needed; inside quotes the template already has (`"{path}"`)
/// it is only escaped. Unknown `{…}` stay as they are.
pub fn expand(template: &str, v: &Values<'_>) -> String {
    fill(template, v, true)
}

/// Fill in the placeholders without any quoting (for the working folder).
pub fn expand_plain(template: &str, v: &Values<'_>) -> String {
    fill(template, v, false)
}

fn fill(template: &str, v: &Values<'_>, quoting: bool) -> String {
    let mut out = String::with_capacity(template.len() + 64);
    let mut in_quotes = false;
    let mut backslashes = 0;
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        if c == '{' {
            if let Some(end) = rest.find('}') {
                if let Some(vals) = values(&rest[1..end], v) {
                    let after = &rest[end + 1..];
                    if !quoting {
                        out.push_str(&vals.join(" "));
                    } else if in_quotes {
                        let quote_follows = after.starts_with('"');
                        for (i, val) in vals.iter().enumerate() {
                            if i > 0 {
                                out.push(' ');
                            }
                            escape_into(&mut out, val, quote_follows && i + 1 == vals.len());
                        }
                    } else {
                        out.push_str(&vals.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" "));
                    }
                    backslashes = 0;
                    rest = after;
                    continue;
                }
            }
        }
        match c {
            '\\' => backslashes += 1,
            '"' => {
                if backslashes % 2 == 0 {
                    in_quotes = !in_quotes;
                }
                backslashes = 0;
            }
            _ => backslashes = 0,
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// The arguments of a command line, split like `CommandLineToArgvW` does
/// (after the program name).
pub fn split_args(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut args = Vec::new();
    let mut i = 0;
    loop {
        while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
            i += 1;
        }
        if i >= chars.len() {
            return args;
        }
        let mut arg = String::new();
        let mut in_quotes = false;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                let start = i;
                while i < chars.len() && chars[i] == '\\' {
                    i += 1;
                }
                let n = i - start;
                if i < chars.len() && chars[i] == '"' {
                    arg.extend(std::iter::repeat_n('\\', n / 2));
                    if n % 2 == 1 {
                        arg.push('"');
                        i += 1;
                    }
                } else {
                    arg.extend(std::iter::repeat_n('\\', n));
                }
                continue;
            }
            if c == '"' {
                if in_quotes && chars.get(i + 1) == Some(&'"') {
                    // `""` inside quotes is a quote.
                    arg.push('"');
                    i += 2;
                } else {
                    in_quotes = !in_quotes;
                    i += 1;
                }
                continue;
            }
            if !in_quotes && (c == ' ' || c == '\t') {
                break;
            }
            arg.push(c);
            i += 1;
        }
        args.push(arg);
    }
}

/// The program of a command line and what follows it, by the rules for the
/// first argument: up to the closing quote if it starts with one, else up
/// to the first space.
pub fn split_program(line: &str) -> (String, &str) {
    let line = line.trim_start();
    if let Some(quoted) = line.strip_prefix('"') {
        return match quoted.find('"') {
            Some(end) => (quoted[..end].to_owned(), &quoted[end + 1..]),
            None => (quoted.to_owned(), ""),
        };
    }
    let end = line.find([' ', '\t']).unwrap_or(line.len());
    (line[..end].to_owned(), &line[end..])
}

/// Whether `template` needs `cmd /C`: pipes and redirection (`|`, `&`, `<`,
/// `>`, `^` outside quotes), `%VARIABLES%`, or a built-in command of cmd
/// (`echo`, `dir`, `start`, …) as the program.
pub fn needs_shell(template: &str) -> bool {
    let mut in_quotes = false;
    for c in template.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            '|' | '&' | '<' | '>' | '^' if !in_quotes => return true,
            _ => {}
        }
    }
    let mut parts = template.split('%');
    parts.next();
    let mut parts = parts.peekable();
    while let Some(part) = parts.next() {
        let is_var = parts.peek().is_some()
            && part.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && !part.contains([' ', '"']);
        if is_var {
            return true;
        }
    }
    let (program, _) = split_program(template);
    let program = program.to_lowercase();
    BUILTINS.contains(&program.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_for(template: &str, items: &[&str], folder: &str) -> String {
        let items: Vec<PathBuf> = items.iter().map(PathBuf::from).collect();
        expand(
            template,
            &Values {
                items: &items,
                folder: Path::new(folder),
            },
        )
    }

    #[test]
    fn quoting_follows_the_windows_rules() {
        for (raw, quoted) in [
            (r"C:\plain\file.txt", r"C:\plain\file.txt"),
            (r"C:\My Files\a.txt", r#""C:\My Files\a.txt""#),
            (r"C:\My Files\", r#""C:\My Files\\""#),
            (r"C:\", r"C:\"),
            (r"D:\a b\\", r#""D:\a b\\\\""#),
            (r#"say "hi""#, r#""say \"hi\"""#),
            (r#"a\"b"#, r#""a\\\"b""#),
            ("", "\"\""),
            (r"C:\Tom&Jerry\x", r#""C:\Tom&Jerry\x""#),
            (r"C:\(x86)\y", r#""C:\(x86)\y""#),
            ("C:\\Bilder\\Größe ü.png", "\"C:\\Bilder\\Größe ü.png\""),
            ("日本語", "日本語"),
        ] {
            assert_eq!(quote(raw), quoted, "{raw}");
            assert_eq!(split_args(&quote(raw)), vec![raw.to_owned()], "{raw}");
        }
    }

    #[test]
    fn split_args_like_command_line_to_argv() {
        assert_eq!(
            split_args(r#"a "b c" d\"e "f\\" g\\h "x""y" """#),
            vec!["a", "b c", "d\"e", "f\\", "g\\\\h", "x\"y", ""]
        );
        assert_eq!(split_args("  "), Vec::<String>::new());
        assert_eq!(split_args(r#""My song".mp3"#), vec!["My song.mp3"]);
    }

    #[test]
    fn placeholders() {
        let items = [r"C:\Music\My song.flac", r"C:\Music\b.mp3"];
        let f = r"C:\Music";
        assert_eq!(
            expand_for("ffmpeg -i {path} {stem}.mp3", &items[..1], f),
            r#"ffmpeg -i "C:\Music\My song.flac" "My song".mp3"#
        );
        assert_eq!(
            split_args(&expand_for("-i {path} {stem}.mp3", &items[..1], f)),
            vec!["-i", r"C:\Music\My song.flac", "My song.mp3"]
        );
        assert_eq!(
            expand_for("x {paths} {n}", &items, f),
            r#"x "C:\Music\My song.flac" C:\Music\b.mp3 2"#
        );
        assert_eq!(
            expand_for("{selection}", &items, f),
            expand_for("{paths}", &items, f)
        );
        assert_eq!(
            expand_for(
                "{name}|{ext}|{parent}|{folder}",
                &items[1..],
                r"D:\Other Dir"
            ),
            r#"b.mp3|mp3|C:\Music|"D:\Other Dir""#
        );
        // Unknown names and braces of other languages stay.
        assert_eq!(
            expand_for("ps -c \"% { $_.Hash }\" {nope} {path", &items[1..], f),
            "ps -c \"% { $_.Hash }\" {nope} {path"
        );
        // A file without extension (an empty value is an empty argument), a drive root.
        assert_eq!(
            expand_for("{stem} {ext}", &["/a/Makefile"], "/a"),
            "Makefile \"\""
        );
        assert_eq!(expand_for("{name} {parent}", &[r"C:\"], r"C:\"), r"C:\ C:\");
    }

    #[test]
    fn placeholders_inside_quotes_of_the_template() {
        let f = r"C:\x";
        // Already quoted: not quoted again.
        assert_eq!(
            expand_for(r#"code "{path}""#, &[r"C:\My Files\a.txt"], f),
            r#"code "C:\My Files\a.txt""#
        );
        // A trailing backslash before the closing quote is doubled …
        let line = expand_for(r#"tool "{path}""#, &[r"C:\My Files\"], f);
        assert_eq!(line, r#"tool "C:\My Files\\""#);
        assert_eq!(split_args(&line), vec!["tool", r"C:\My Files\"]);
        // … but not when more text follows inside the quotes.
        let line = expand_for(r#"tool "{folder}sub dir""#, &[r"C:\a"], r"C:\My Files\");
        assert_eq!(split_args(&line), vec!["tool", r"C:\My Files\sub dir"]);
        // Escaped quotes don't end the quoted part.
        let line = expand_for(r#"tool "a \" {name}""#, &["/p/x y"], f);
        assert_eq!(split_args(&line), vec!["tool", "a \" x y"]);
    }

    #[test]
    fn many_items() {
        let items: Vec<String> = (0..500).map(|i| format!(r"C:\Dir {i}\f{i}.txt")).collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        let line = expand_for("tool {paths}", &refs, r"C:\");
        let args = split_args(&line);
        assert_eq!(args.len(), 501);
        assert_eq!(args[1..], items[..]);
        assert_eq!(expand_for("{n}", &refs, r"C:\"), "500");
    }

    #[test]
    fn plain_expansion_for_folders() {
        let items = [PathBuf::from(r"C:\My Files\a.txt")];
        let v = Values {
            items: &items,
            folder: Path::new(r"C:\"),
        };
        assert_eq!(expand_plain(r"{parent}\out", &v), r"C:\My Files\out");
    }

    #[test]
    fn program_and_shell() {
        assert_eq!(split_program("code {paths}"), ("code".into(), " {paths}"));
        assert_eq!(
            split_program(r#"  "C:\Program Files\x.exe" -a"#),
            (r"C:\Program Files\x.exe".into(), " -a")
        );
        assert_eq!(split_program(r#""C:\open"#), (r"C:\open".into(), ""));
        assert!(!needs_shell("code {paths}"));
        assert!(
            !needs_shell(r#"powershell -Command "a | b > c""#),
            "inside quotes"
        );
        assert!(needs_shell("certutil -hashfile {path} | clip"));
        assert!(needs_shell("tool {path} && pause"));
        assert!(needs_shell("tool > out.txt"));
        assert!(needs_shell("tool %USERPROFILE%\\x"));
        assert!(!needs_shell("tool 50% done"));
        assert!(!needs_shell("tool \"50% of 100%\""));
        assert!(needs_shell("echo {name}"));
        assert!(needs_shell("START \"\" {path}"));
        assert!(!needs_shell("echoes {name}"));
        assert!(!needs_shell(r#""C:\Tools\dir" {path}"#));
    }
}
