//! A program may hold more than 255 code objects.

use std::process::Command;

#[test]
fn calls_reach_code_objects_past_255() {
    let dir = std::env::temp_dir().join(format!("mote_many_functions_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut chain = String::from("pub var BASE = 1\nfn f0(x: Int) -> Int { return x + BASE }\n");
    for i in 1..300 {
        chain.push_str(&format!("fn f{i}(x: Int) -> Int {{ return f{}(x) + 1 }}\n", i - 1));
    }
    chain.push_str("pub fn top(x: Int) -> Int { return f299(x) }\n");
    std::fs::write(dir.join("chain.mote"), chain).unwrap();

    let mut main = String::from("import .chain\n");
    for i in 0..300 {
        main.push_str(&format!("fn g{i}() -> Int {{ return {i} }}\n"));
    }
    main.push_str(
        "class Box {\n    v: Int\n    fn get(self) -> Int { return self.v }\n}\n\
         fn pad(x: Int, y: Int = g299()) -> Int { return x + y }\n\
         fn total(...xs: Int) -> Int {\n    var s = 0\n    for x in xs { s = s + x }\n    return s\n}\n\
         fn same<T>(x: T) -> T { return x }\n\
         fn main() {\n    let add = |a: Int| a + g298()\n    println(top(0))\n    println(Box { v: g297() }.get())\n    println(pad(1))\n    println(total(g1(), g2(), g3()))\n    println(same(add(2)))\n    println(same(\"s\"))\n}\n",
    );
    std::fs::write(dir.join("main.mote"), main).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && text == "300\n297\n300\n6\n300\ns\n", "{text}");
}
