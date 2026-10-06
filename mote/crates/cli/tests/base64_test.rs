//! `std.data.base64`.

use std::process::Command;

fn run(source: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("mote_base64_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text
}

const HEAD: &str = r#"import std.data.base64 as base64

fn back(r: Result<Bytes, Error>) -> String {
    match r {
        Ok(b) => { return "ok:" + b.decode().unwrap() }
        Err(e) => { return "err:" + e.message }
    }
}

fn same(a: Bytes, b: Bytes) -> Bool {
    if a.len() != b.len() { return false }
    var i = 0
    while i < a.len() {
        if a.get(i) != b.get(i) { return false }
        i = i + 1
    }
    return true
}
"#;

fn lines(body: &str, tag: &str) -> Vec<String> {
    run(&format!("{HEAD}\nfn main() {{\n{body}}}\n"), tag).lines().map(String::from).collect()
}

#[test]
fn the_rfc_4648_vectors_encode() {
    let body = ["", "f", "fo", "foo", "foob", "fooba", "foobar"].iter().map(|s| format!("    println(base64.encode(\"{s}\".bytes()))\n")).collect::<String>();
    assert_eq!(lines(&body, "encode"), ["", "Zg==", "Zm8=", "Zm9v", "Zm9vYg==", "Zm9vYmE=", "Zm9vYmFy"]);
}

#[test]
fn the_rfc_4648_vectors_decode() {
    let body = ["", "Zg==", "Zm8=", "Zm9v", "Zm9vYg==", "Zm9vYmE=", "Zm9vYmFy"].iter().map(|s| format!("    println(back(base64.decode(\"{s}\")))\n")).collect::<String>();
    assert_eq!(lines(&body, "decode"), ["ok:", "ok:f", "ok:fo", "ok:foo", "ok:foob", "ok:fooba", "ok:foobar"]);
}

#[test]
fn bad_text_is_an_error() {
    let body = ["Zg", "Zg=", "Z===", "Zm9v!A==", "Zg=a", "Zm9v Zm9v", "Zg==Zg=="]
        .iter()
        .map(|s| format!("    println(back(base64.decode(\"{s}\")))\n"))
        .collect::<String>();
    assert_eq!(
        lines(&body, "bad"),
        [
            "err:Invalid padding",
            "err:Invalid padding",
            "err:Invalid symbol 61, offset 1.",
            "err:Invalid symbol 33, offset 4.",
            "err:Invalid symbol 61, offset 2.",
            "err:Invalid symbol 32, offset 4.",
            "err:Invalid symbol 61, offset 2.",
        ]
    );
}

#[test]
fn every_byte_value_round_trips_in_both_alphabets() {
    let body = "    var all = Bytes()
    var i = 0
    while i < 256 {
        all.push(i)
        i = i + 1
    }
    let e = base64.encode(all)
    let u = base64.encode_url(all)
    println(same(base64.decode(e).unwrap(), all))
    println(same(base64.decode_url(u).unwrap(), all))
    println(e.contains(\"+\") && e.contains(\"/\") && e.ends_with(\"=\"))
    println(u.contains(\"-\") && u.contains(\"_\") && !u.contains(\"=\") && !u.contains(\"+\") && !u.contains(\"/\"))
    println(e.len())
    println(u.len())
";
    assert_eq!(lines(body, "round"), ["true", "true", "true", "true", "344", "342"]);
}

#[test]
fn the_url_alphabet_has_no_padding_and_rejects_the_standard_one() {
    let body = "    println(base64.encode_url(\"f\".bytes()))
    println(base64.encode_url(\"fo\".bytes()))
    println(back(base64.decode_url(\"Zg\")))
    println(back(base64.decode_url(\"Zg==\")))
    println(back(base64.decode_url(\"Zm9v+/\")))
    println(back(base64.decode_url(\"Z\")))
    println(back(base64.decode(\"-_-_\")))
";
    assert_eq!(
        lines(body, "url"),
        [
            "Zg",
            "Zm8",
            "ok:f",
            "err:Invalid padding",
            "err:Invalid symbol 43, offset 4.",
            "err:Invalid input length: 1",
            "err:Invalid symbol 45, offset 0.",
        ]
    );
}

#[test]
fn bytes_that_are_not_text_decode_to_bytes() {
    let body = "    let b = base64.decode(\"/w==\").unwrap()
    println(b.len())
    println(b.get(0))
";
    assert_eq!(lines(body, "binary"), ["1", "255"]);
}
