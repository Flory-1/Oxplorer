#!/usr/bin/env bash
# PostToolUse hook for Edit/Write: keeps source files within the line budget.
# A file may not grow past LIMIT lines. Files already over it (from before the
# budget) may be edited, but may not get longer than they are in HEAD.
# Exit 2 sends the message back to Claude; exit 0 stays silent.

LIMIT=1000

input=$(cat)
path=$(printf '%s' "$input" | grep -o '"file_path"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 \
  | sed 's/^"file_path"[[:space:]]*:[[:space:]]*"//; s/"$//; s/\\\\/\//g')
[ -n "$path" ] && [ -f "$path" ] || exit 0

case "$path" in
  *.rs|*.toml|*.py|*.ps1|*.iss|*.yml|*.yaml|*.md) ;;
  *) exit 0 ;;
esac
case "$path" in
  */Cargo.lock|*/CHANGELOG.md|*/i18n_de.rs) exit 0 ;;
esac

lines=$(wc -l < "$path" | tr -d ' ')
[ "$lines" -gt "$LIMIT" ] || exit 0

dir=$(dirname "$path")
base=$(cd "$dir" && git show "HEAD:./$(basename "$path")" 2>/dev/null | wc -l | tr -d ' ')
[ -n "$base" ] || base=0

if [ "$base" -gt "$LIMIT" ] && [ "$lines" -le "$base" ]; then
  exit 0
fi

# Newlines in a JSON string field of the hook input (approximate).
newlines() {
  printf '%s' "$input" | grep -oE "\"$1\"[[:space:]]*:[[:space:]]*\"(\\\\.|[^\"\\\\])*\"" | head -1 \
    | grep -o '\\n' | wc -l | tr -d ' '
}
# An edit that adds no lines is fine, even to a file that grew since HEAD.
if printf '%s' "$input" | grep -q '"old_string"'; then
  [ "$(newlines new_string)" -gt "$(newlines old_string)" ] || exit 0
fi

if [ "$base" -gt "$LIMIT" ]; then
  echo "Line budget: $path is $lines lines (HEAD: $base, budget: $LIMIT). It is already over budget and must not grow. Move code into a submodule instead (see the file-splitter agent)." >&2
else
  echo "Line budget: $path is $lines lines (budget: $LIMIT). Split it into submodules so every file stays at or under $LIMIT lines." >&2
fi
exit 2
