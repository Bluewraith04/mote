//! `std.data.crypto` against published test vectors.

use std::process::Command;

fn run(body: &str, tag: &str) -> Vec<String> {
    let source = format!("import std.data.crypto as crypto\nimport std.data as data\n\nfn main() {{\n{body}}}\n");
    let dir = std::env::temp_dir().join(format!("mote_crypto_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text.lines().map(String::from).collect()
}

#[test]
fn sha2_hashes_match_the_published_vectors() {
    let got = run(
        "    println(crypto.to_hex(crypto.sha256(Bytes())))
    println(crypto.to_hex(crypto.sha256(\"abc\".bytes())))
    println(crypto.to_hex(crypto.sha384(\"abc\".bytes())))
    println(crypto.to_hex(crypto.sha512(\"abc\".bytes())))
    println(crypto.sha256(\"abc\".bytes()).len())
    println(crypto.sha384(\"abc\".bytes()).len())
    println(crypto.sha512(\"abc\".bytes()).len())
",
        "sha",
    );
    assert_eq!(
        got,
        [
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            "32",
            "48",
            "64",
        ]
    );
}

#[test]
fn hmac_matches_rfc_4231() {
    let got = run(
        "    let key = Bytes(20)
    var i = 0
    while i < 20 {
        key.set(i, 11)
        i = i + 1
    }
    let hi = \"Hi There\".bytes()
    println(crypto.to_hex(crypto.hmac_sha256(key, hi)))
    println(crypto.to_hex(crypto.hmac_sha384(key, hi)))
    println(crypto.to_hex(crypto.hmac_sha512(key, hi)))
    println(crypto.to_hex(crypto.hmac_sha256(\"Jefe\".bytes(), \"what do ya want for nothing?\".bytes())))
    println(crypto.to_hex(crypto.hmac_sha256(Bytes(), Bytes())))
",
        "hmac",
    );
    assert_eq!(
        got,
        [
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            "afd03944d84895626b0825f4ab46907f15f9dadbe4101ec682aa034c7cebc59cfaea9ea9076ede7f4af152e8b2fa9cb6",
            "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854",
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad",
        ]
    );
}

#[test]
fn hex_and_comparison_helpers() {
    let got = run(
        "    println(crypto.to_hex(\"Mote\".bytes()))
    println(crypto.from_hex(\"4D6f7465\").unwrap().decode().unwrap())
    println(crypto.from_hex(\"\").unwrap().len())
    println(crypto.from_hex(\"abc\").is_err())
    println(crypto.from_hex(\"zz\").is_err())
    println(crypto.constant_time_eq(\"tag\".bytes(), \"tag\".bytes()))
    println(crypto.constant_time_eq(\"tag\".bytes(), \"tab\".bytes()))
    println(crypto.constant_time_eq(\"tag\".bytes(), \"tags\".bytes()))
    println(crypto.constant_time_eq(Bytes(), Bytes()))
    println(data.crypto.to_hex(\"a\".bytes()))
",
        "hex",
    );
    assert_eq!(got, ["4d6f7465", "Mote", "0", "true", "true", "true", "false", "false", "true", "61"]);
}

#[test]
fn random_bytes_are_the_right_size_and_differ() {
    let got = run(
        "    println(crypto.random_bytes(0).unwrap().len())
    println(crypto.random_bytes(32).unwrap().len())
    println(crypto.random_bytes(1000000).unwrap().len())
    let a = crypto.random_bytes(32).unwrap()
    let b = crypto.random_bytes(32).unwrap()
    println(crypto.constant_time_eq(a, b))
    println(crypto.random_bytes(-1).is_err())
    println(crypto.random_bytes(67108865).is_err())
",
        "random",
    );
    assert_eq!(got, ["0", "32", "1000000", "false", "true", "true"]);
}

#[test]
fn passwords_hash_with_a_fresh_salt_and_verify() {
    let got = run(
        "    let a = crypto.hash_password(\"correct horse\").unwrap()
    let b = crypto.hash_password(\"correct horse\").unwrap()
    println(a.slice(0, 31))
    println(a == b)
    println(crypto.verify_password(\"correct horse\", a).unwrap())
    println(crypto.verify_password(\"correct horse\", b).unwrap())
    println(crypto.verify_password(\"Correct horse\", a).unwrap())
    println(crypto.verify_password(\"\", a).unwrap())
    let e = crypto.hash_password(\"\").unwrap()
    println(crypto.verify_password(\"\", e).unwrap())
    let u = crypto.hash_password(\"pässwörd ✓\").unwrap()
    println(crypto.verify_password(\"pässwörd ✓\", u).unwrap())
    match crypto.verify_password(\"x\", \"not a hash\") {
        Ok(v) => { println(\"ok\") }
        Err(err) => { println(err.kind == ErrorKind.InvalidData) }
    }
    println(crypto.verify_password(\"x\", \"$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\").unwrap())
",
        "password",
    );
    assert_eq!(got, ["$argon2id$v=19$m=19456,t=2,p=1$", "false", "true", "true", "false", "false", "true", "true", "true", "false"]);
}
