#!/usr/bin/env python3
"""Vanillish: the JNI symbols the app's Java sources expect libvanillish.so to export.

    android/tools/jni-natives.py android/app/src/main/java      # one symbol per line, sorted

Every `native` method a Java class declares is looked up by name in the loaded library the first
time it is called (`Java_<class>_<method>`, JNI's "short name"); a missing one is not a link or
load error but an `UnsatisfiedLinkError` thrown on that first call, on whatever thread made it —
a controller plugged in mid-game, a picker returning. `android/build-rust.sh` compares this list
with the library's dynamic symbol table after every build, so a rename on one side, or a
`static native` added in Java without its `#[no_mangle]` export in
`crates/vanillish-android/src/jni_exports.rs`, fails the build instead.

The scan is lexical, not a Java parser: comments, string and character literals are blanked
first (so a comment about "the native thread" is not a declaration), then class bodies are
followed by their braces so a nested class's natives get its binary name (`Outer$Inner`, mangled
`Outer_00024Inner`). Overloaded natives would need JNI's long names (`__` + the argument
signature); this reports them as an error rather than guessing, as the app declares none.

Standard library only (python3 on Linux and macOS), so the build needs nothing installed for it.
"""

import os
import re
import sys

# A Java identifier: a letter (any script), `_` or `$`, then letters, digits, `_` or `$`.
IDENT = r"(?:[^\W\d]|\$)[\w$]*"
KEYWORD_DECL = re.compile(r"(?<![\w$.@])(class|interface|enum|record)\s+(" + IDENT + ")")
ANNOTATION_DECL = re.compile(r"@\s*interface\s+(" + IDENT + ")")
# `native` up to the first `(`: the identifier right before it is the method's name. Return types
# never contain `(`, `;`, `{` or `}`, so the lazy match cannot run into the next member.
NATIVE = re.compile(r"(?<![\w$.])native\b[^;{}()=]*?(?<![\w$])(" + IDENT + r")\s*\(")
# Java translates `\uXXXX` escapes before it reads anything else, identifiers included.
UNICODE_ESCAPE = re.compile(r"(?<!\\)\\u+([0-9a-fA-F]{4})")
PACKAGE = re.compile(r"^\s*package\s+([\w.]+)\s*;", re.M)


def blank_comments_and_literals(src):
    """`src` with comments and string/char literals replaced by spaces (newlines kept)."""
    out = []
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append(re.sub(r"[^\n]", " ", src[i:j]))
            i = j
        elif src.startswith('"""', i):
            j = src.find('"""', i + 3)
            while j > 0 and src[j - 1] == "\\":
                j = src.find('"""', j + 1)
            j = n if j < 0 else j + 3
            out.append(re.sub(r"[^\n]", " ", src[i:j]))
            i = j
        elif c in "\"'":
            j = i + 1
            while j < n and src[j] != c and src[j] != "\n":
                j += 2 if src[j] == "\\" else 1
            j = min(j + 1, n)
            out.append(" " * (j - i))
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


def mangle(name):
    """JNI's escaping of one name component (JNI spec, "Resolving Native Method Names")."""
    out = []
    for ch in name:
        if ch == "_":
            out.append("_1")
        elif ch == ";":
            out.append("_2")
        elif ch == "[":
            out.append("_3")
        elif ch.isascii() and ch.isalnum():
            out.append(ch)
        else:
            out.append("_0%04x" % ord(ch))
    return "".join(out)


def natives(path):
    """[(binary class name with dots and $, method name)] declared in one .java file."""
    with open(path, encoding="utf-8") as f:
        src = f.read()
    src = UNICODE_ESCAPE.sub(lambda m: chr(int(m.group(1), 16)), src)
    src = blank_comments_and_literals(src)
    m = PACKAGE.search(src)
    package = m.group(1) if m else ""

    # The `{` that opens each named type's body.
    opens = {}
    decls = [(d.start(), d.group(2)) for d in KEYWORD_DECL.finditer(src)]
    decls += [(d.start(), d.group(1)) for d in ANNOTATION_DECL.finditer(src)]
    for start, name in decls:
        brace = src.find("{", start)
        if brace >= 0:
            opens.setdefault(brace, name)

    found = []
    native_at = {m.start(): m.group(1) for m in NATIVE.finditer(src)}
    stack = []  # [(name, depth of its body)]
    depth = 0
    for i, c in enumerate(src):
        if i in native_at:
            if not stack:
                raise SystemExit(f"{path}: a native method outside any class")
            binary = "$".join(name for name, _ in stack)
            found.append((f"{package}.{binary}" if package else binary, native_at[i]))
        if c == "{":
            depth += 1
            if i in opens:
                stack.append((opens[i], depth))
        elif c == "}":
            if stack and stack[-1][1] == depth:
                stack.pop()
            depth -= 1
    return found


def symbol(cls, method):
    parts = [mangle(p) for p in cls.split(".")]
    return "Java_" + "_".join(parts) + "_" + mangle(method)


def main(argv):
    if len(argv) < 2:
        print(__doc__.strip().splitlines()[2].strip(), file=sys.stderr)
        return 2
    symbols = {}
    for root in argv[1:]:
        for dirpath, _, files in os.walk(root):
            for name in sorted(files):
                if not name.endswith(".java"):
                    continue
                path = os.path.join(dirpath, name)
                for cls, method in natives(path):
                    symbols.setdefault(symbol(cls, method), []).append(f"{cls}.{method} ({path})")
    overloaded = {s: w for s, w in symbols.items() if len(w) > 1}
    for s, where in sorted(overloaded.items()):
        print(f"jni-natives: overloaded native {s}: " + "; ".join(where), file=sys.stderr)
    if overloaded:
        print("jni-natives: overloaded natives need JNI long names; give each a distinct name",
              file=sys.stderr)
        return 1
    for s in sorted(symbols):
        print(s)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
