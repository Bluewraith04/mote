//! A `Map` or `Set` keeps entries in insertion order behind a small index; behaviour is unchanged.

use std::process::Command;

fn run(body: &str, tag: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("mote_table_{}_{}", tag, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.mote"), format!("fn main() {{\n{body}}}\n")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_mote")).arg("run").arg(dir.join("main.mote")).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text.lines().map(String::from).collect()
}

#[test]
fn a_set_iterates_in_insertion_order() {
    let out = run(
        "    var s: Set<Int> = Set()\n    for k in [50, 3, 91, 7, 3, 12] {\n        s.add(k)\n    }\n    println(s)\n    println(s.len())\n    println(s.items())\n",
        "set_order",
    );
    assert_eq!(out, ["{50, 3, 91, 7, 12}", "5", "[50, 3, 91, 7, 12]"]);
}

#[test]
fn removing_and_readding_moves_a_key_to_the_end() {
    let out = run(
        "    var m: Map<String, Int> = {\"a\": 1, \"b\": 2, \"c\": 3}\n    m.remove(\"a\")\n    m[\"a\"] = 10\n    m[\"b\"] = 20\n    println(m)\n    println(m.keys())\n    println(m.values())\n",
        "readd",
    );
    assert_eq!(out, ["{\"b\": 20, \"c\": 3, \"a\": 10}", "[\"b\", \"c\", \"a\"]", "[20, 3, 10]"]);
}

#[test]
fn many_entries_survive_growth_removal_and_compaction() {
    let out = run(
        "    var m: Map<Int, Int> = Map()\n    var i = 0\n    while i < 5000 {\n        m[i] = i * 2\n        i += 1\n    }\n    i = 0\n    while i < 5000 {\n        if i % 3 != 0 {\n            m.remove(i)\n        }\n        i += 1\n    }\n    println(m.len())\n    println(m[0])\n    println(m[4998])\n    println(m.contains_key(1))\n    println(m.contains_key(4998))\n    i = 5000\n    while i < 6000 {\n        m[i] = i\n        i += 1\n    }\n    println(m.len())\n    println(m[5999])\n    println(m.keys()[0])\n",
        "grow",
    );
    assert_eq!(out, ["1667", "0", "9996", "false", "true", "2667", "5999", "0"]);
}

#[test]
fn churning_distinct_keys_through_a_small_map_stays_correct() {
    let out = run(
        "    var m: Map<Int, Int> = Map()\n    var i = 0\n    while i < 20000 {\n        m[i] = i\n        m.remove(i - 3)\n        i += 1\n    }\n    println(m.len())\n    println(m.keys())\n    println(m.contains_key(5))\n    println(m[19999])\n",
        "churn",
    );
    assert_eq!(out, ["3", "[19997, 19998, 19999]", "false", "19999"]);
}

#[test]
fn a_cleared_table_is_reusable() {
    let out = run(
        "    var s: Set<String> = Set()\n    s.add(\"x\")\n    s.add(\"y\")\n    s.clear()\n    println(s.len())\n    println(s.contains(\"x\"))\n    s.add(\"z\")\n    println(s)\n    var m: Map<Int, Int> = Map()\n    var i = 0\n    while i < 300 {\n        m[i] = i\n        i += 1\n    }\n    m.clear()\n    m[7] = 1\n    println(m)\n",
        "clear",
    );
    assert_eq!(out, ["0", "false", "{\"z\"}", "{7: 1}"]);
}

#[test]
fn equality_ignores_insertion_order_and_removal_history() {
    let out = run(
        "    var a: Map<Int, Int> = Map()\n    a[1] = 10\n    a[2] = 20\n    a[3] = 30\n    a.remove(2)\n    var b: Map<Int, Int> = Map()\n    b[3] = 30\n    b[1] = 10\n    println(a == b)\n    b[1] = 11\n    println(a == b)\n    var s: Set<Int> = Set()\n    s.add(1)\n    s.add(2)\n    var t: Set<Int> = Set()\n    t.add(2)\n    t.add(1)\n    println(s == t)\n",
        "equal",
    );
    assert_eq!(out, ["true", "false", "true"]);
}

#[test]
fn a_map_crosses_a_task_boundary() {
    let out = run(
        "    scope {\n        let t = spawn {\n            var m: Map<String, Int> = Map()\n            m[\"one\"] = 1\n            m[\"two\"] = 2\n            m.remove(\"one\")\n            m[\"three\"] = 3\n            return m\n        }\n        let got = t.join().unwrap()\n        println(got)\n        println(got[\"three\"])\n    }\n",
        "task",
    );
    assert_eq!(out, ["{\"two\": 2, \"three\": 3}", "3"]);
}

#[test]
fn a_missing_key_and_a_null_key_behave_as_before() {
    let out = run(
        "    var m: Map<String, Int> = Map()\n    println(m.get_or(\"x\", -1))\n    println(m.contains_key(\"x\"))\n    println(m.remove(\"x\"))\n    var s: Set<Int> = Set()\n    println(s.remove(1))\n    println(s.add(1))\n    println(s.add(1))\n",
        "missing",
    );
    assert_eq!(out, ["-1", "false", "false", "false", "true", "false"]);
}
