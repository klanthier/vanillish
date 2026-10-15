#!/usr/bin/env python3
"""Vanillish: tests for android/tools/jni-natives.py, and the Java <-> Rust JNI contract.

    python3 android/tools/test_jni_natives.py        # stdlib unittest; vanillish-check.yml runs it

build-rust.sh compares jni-natives.py's list with the built library's exports, but only a full
Android build gets there (vanillish-apk.yml, minutes of Actions time). The last test here reads the
same contract from the sources instead — every Java `native` has its `#[no_mangle]` function in
crates/vanillish-android/src/jni_exports.rs — so a rename on one side fails the push's check.
"""

import contextlib
import importlib.util
import io
import os
import re
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))

_spec = importlib.util.spec_from_file_location("jni_natives", os.path.join(HERE, "jni-natives.py"))
jni = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(jni)


def scan(source):
    """The symbols jni-natives.py derives from one Java file with `source` as its text."""
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "T.java")
        with open(path, "w", encoding="utf-8") as f:
            f.write(source)
        return sorted(jni.symbol(cls, method) for cls, method in jni.natives(path))


class Mangle(unittest.TestCase):
    def test_escapes(self):
        # JNI spec, "Resolving Native Method Names": _ → _1, ; → _2, [ → _3, else _0xxxx.
        self.assertEqual(jni.mangle("plain9"), "plain9")
        self.assertEqual(jni.mangle("a_b"), "a_1b")
        self.assertEqual(jni.mangle("Outer$Inner"), "Outer_00024Inner")
        self.assertEqual(jni.mangle("café"), "caf_000e9")
        self.assertEqual(jni.mangle("x;y[z"), "x_2y_3z")

    def test_symbol(self):
        self.assertEqual(
            jni.symbol("io.github.klanthier.vanillish.GamepadBridge", "nativePadKey"),
            "Java_io_github_klanthier_vanillish_GamepadBridge_nativePadKey",
        )
        self.assertEqual(jni.symbol("a.b_c.D$E", "f_g"), "Java_a_b_1c_D_00024E_f_1g")
        self.assertEqual(jni.symbol("Top", "m"), "Java_Top_m")


class Scan(unittest.TestCase):
    def test_declarations_not_lookalikes(self):
        src = r'''
// package fake.pkg; native void commented();
package a.b;

/* class Fake { static native void inComment(); } */
public final class T {
    static native void top(int x);
    static final String S = "native void inString(";
    static final char C = '"';
    static final String Q = "\" native void escaped(";
    public static native <X> java.util.List<X>[] generic(X x);
    native int with_underscore();
    native void unicode();
    @Deprecated
    private static native String annotated(String s);
    static Runnable r = new Runnable() { public void run() { boolean isNative = true; } };
    Class<?> k = T.class;
    String tb = """
        native void inTextBlock(
        """;
    native void last();
}
'''
        self.assertEqual(
            scan(src),
            [
                "Java_a_b_T_annotated",
                "Java_a_b_T_generic",
                "Java_a_b_T_last",
                "Java_a_b_T_top",
                "Java_a_b_T_unicode",
                "Java_a_b_T_with_1underscore",
            ],
        )

    def test_nested_types_get_binary_names(self):
        src = """
package p;
class Outer {
    static class Inner {
        static native void inner();
        enum E { A { void f() {} }, B; static native void inEnum(); }
    }
    interface I { void notNative(); }
    record R(int a) { static native void inRecord(); }
    native void outerAgain();
}
class Second { native void second(); }
"""
        self.assertEqual(
            scan(src),
            [
                "Java_p_Outer_00024Inner_00024E_inEnum",
                "Java_p_Outer_00024Inner_inner",
                "Java_p_Outer_00024R_inRecord",
                "Java_p_Outer_outerAgain",
                "Java_p_Second_second",
            ],
        )

    def test_overloads_are_refused(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "O.java"), "w", encoding="utf-8") as f:
                f.write("package p; class O { native void f(int a); native void f(long a); }\n")
            with contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(jni.main(["jni-natives.py", d]), 1)
            self.assertIn("overloaded native Java_p_O_f", err.getvalue())


class Contract(unittest.TestCase):
    def test_every_java_native_has_its_rust_export(self):
        java = os.path.join(ROOT, "android", "app", "src")
        declared = set()
        for dirpath, _, files in os.walk(java):
            for name in files:
                if name.endswith(".java"):
                    path = os.path.join(dirpath, name)
                    declared.update(jni.symbol(c, m) for c, m in jni.natives(path))
        self.assertTrue(declared, "no Java natives found under android/app/src")

        with open(os.path.join(ROOT, "crates", "vanillish-android", "src", "jni_exports.rs"),
                  encoding="utf-8") as f:
            rust = f.read()
        # An export no Java declares is only build-rust.sh's warning, so only this direction fails.
        exported = set(re.findall(r'extern\s+"(?:system|C)"\s+fn\s+(Java_\w+)\s*\(', rust))
        self.assertEqual(
            sorted(declared - exported), [],
            "Java natives without a #[no_mangle] pub extern \"system\" fn in jni_exports.rs",
        )


if __name__ == "__main__":
    unittest.main()
