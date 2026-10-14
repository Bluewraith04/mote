//! The modifier pass: what it consumes, what it records and what it adds.

use compiler::ast::{Item, Program};
use compiler::lexer::Lexer;
use compiler::modifiers;
use compiler::parser::Parser;
use compiler::span::Span;

fn parse(src: &str) -> Program {
    let tokens = Lexer::new(src).tokenize().expect("lexes");
    Parser::new(tokens).parse().expect("parses")
}

fn methods_of(program: &Program, name: &str) -> Vec<String> {
    program
        .items
        .iter()
        .find_map(|i| match i {
            Item::Struct(s) if s.name == name => Some(s.methods.iter().map(|m| m.name.clone()).collect()),
            _ => None,
        })
        .expect("the struct is there")
}

/// A host that knows `Display` (id 0) and `Debug` (id 1) and answers canned code.
struct Fake {
    runs: usize,
    /// What `Display` answers as its private part.
    helper: &'static str,
    /// What `Display` answers as its public part.
    display: &'static str,
}

impl Fake {
    fn new() -> Fake {
        Fake { runs: 0, helper: "", display: "fn to_string(self) -> String { return \"p\" }" }
    }
}

impl modifiers::Host for Fake {
    fn find(&mut self, name: &str, _at: Span) -> Result<Option<modifiers::Generator>, modifiers::Diagnostic> {
        let (method, id, module) = match name {
            "Display" => ("to_string", 0, "traits"),
            "Debug" => ("debug", 1, "traits"),
            "Broken" => ("broken", 2, "traits"),
            "Reader" => ("read", 3, "traits"),
            "Aliased" => ("aliased", 4, "json"),
            _ => return Ok(None),
        };
        Ok(Some(modifiers::Generator { trait_name: name.to_string(), module: module.to_string(), methods: vec![method.to_string()], id }))
    }

    fn reads(&mut self, generator: &modifiers::Generator) -> Vec<String> {
        if generator.id == 3 { vec!["arg".to_string()] } else { Vec::new() }
    }

    fn run(&mut self, generator: &modifiers::Generator, decl_json: &str) -> Result<modifiers::Additions, String> {
        self.runs += 1;
        assert!(decl_json.starts_with("{\"kind\":"), "{decl_json}");
        match generator.id {
            0 => Ok(modifiers::Additions { public: self.display.to_string(), private: self.helper.to_string() }),
            1 => Ok(modifiers::Additions { public: "fn debug(self) -> String { return \"d\" }".to_string(), private: String::new() }),
            3 => Ok(modifiers::Additions { public: "fn read(self) -> String { return \"r\" }".to_string(), private: String::new() }),
            4 => Ok(modifiers::Additions { public: "fn aliased(self) -> Int { return __modifier.size(1) }".to_string(), private: String::new() }),
            _ => Err("it cannot".to_string()),
        }
    }
}

fn expand_with(src: &str, host: &mut Fake) -> Result<Program, modifiers::Diagnostic> {
    let mut program = parse(src);
    modifiers::expand(&mut program, host)?;
    Ok(program)
}

#[test]
fn the_parser_expands_nothing() {
    let program = parse("struct P: (Display) {\n    x: Int\n}\n");
    assert!(methods_of(&program, "P").is_empty());
    assert!(program.applied.is_empty());
}

#[test]
fn the_pass_adds_the_methods_and_records_the_application() {
    let program = expand_with("struct P: (Display, Debug) {\n    x: Int\n}\n", &mut Fake::new()).unwrap();
    assert_eq!(methods_of(&program, "P"), ["to_string", "debug"]);
    let names: Vec<&str> = program.applied.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Display", "Debug"]);
    assert!(program.applied.iter().all(|a| a.module == "traits" && a.target == "P"));
}

#[test]
fn a_second_run_changes_nothing() {
    let mut host = Fake::new();
    let mut program = expand_with("struct P: (Display) {\n    x: Int\n}\n", &mut host).unwrap();
    let once = program.clone();
    modifiers::expand(&mut program, &mut host).unwrap();
    assert_eq!(program, once);
    assert_eq!(host.runs, 1);
}

#[test]
fn a_trait_the_host_does_not_know_is_left_alone() {
    let program = expand_with("struct P: (Eq, Hash) {\n    x: Int\n}\n", &mut Fake::new()).unwrap();
    assert!(methods_of(&program, "P").is_empty());
}

#[test]
fn a_listed_trait_with_a_modifier_and_none_of_its_methods_runs_it() {
    let program = expand_with("struct P: (Display, Eq) {\n    x: Int\n}\n", &mut Fake::new()).unwrap();
    assert_eq!(methods_of(&program, "P"), ["to_string"]);
    assert_eq!(program.applied[0].name, "Display");
}

#[test]
fn a_listed_trait_whose_method_is_written_is_left_alone() {
    let mut host = Fake::new();
    let program = expand_with("struct P: (Display) {\n    x: Int\n    fn to_string(self) -> String { return \"mine\" }\n}\n", &mut host).unwrap();
    assert_eq!(methods_of(&program, "P"), ["to_string"]);
    assert!(program.applied.is_empty() && host.runs == 0);
}

#[test]
fn a_modifier_that_refuses_is_reported_at_the_request() {
    let (message, span) = expand_with("struct P: (Broken) {\n    x: Int\n}\n", &mut Fake::new()).unwrap_err();
    assert_eq!(message, "listing `Broken`: it cannot");
    assert_eq!(span.line, 1);
}

#[test]
fn code_that_does_not_parse_is_an_error_and_not_a_crash() {
    let mut host = Fake::new();
    host.display = "fn to_string(self) -> String { return ";
    let (message, _) = expand_with("struct P: (Display) {\n}\n", &mut host).unwrap_err();
    assert!(message.starts_with("`Display` generated code that does not parse"), "{message}");
    host.display = "struct Extra {\n}\n";
    let (message, _) = expand_with("struct P: (Display) {\n}\n", &mut host).unwrap_err();
    assert_eq!(message, "`Display` may only add `fn`s");
}

#[test]
fn a_helper_that_clashes_with_a_method_is_refused() {
    let mut host = Fake::new();
    host.helper = "fn __size(self) -> Int { return 1 }";
    let (message, _) = expand_with("struct P: (Display) {\n    fn __size(self) -> Int { return 2 }\n}\n", &mut host).unwrap_err();
    assert_eq!(message, "`P` already has a method named `__size`, so listing `Display` cannot add one");
}

#[test]
fn generated_code_reaches_its_module_through_a_per_module_alias() {
    let program = expand_with("struct P: (Aliased) {\n    x: Int\n}\n", &mut Fake::new()).unwrap();
    let import = program.items.iter().find_map(|i| match i {
        Item::Import(imp) => Some(imp),
        _ => None,
    });
    let import = import.expect("an alias import was added");
    assert_eq!(import.alias.as_deref(), Some("__m_json"));
    assert_eq!(import.path.segments, ["std", "json"]);
    let code = &program.applied[0].code;
    assert!(code.contains("__m_json.") && !code.contains("__modifier."), "{code}");
}

#[test]
fn a_modifier_that_never_names_its_module_adds_no_import() {
    let program = expand_with("struct P: (Display) {\n    x: Int\n}\n", &mut Fake::new()).unwrap();
    assert!(!program.items.iter().any(|i| matches!(i, Item::Import(_))));
}

#[test]
fn derive_is_gone_and_says_what_to_write() {
    assert_eq!(rejected("@derive(Display)\nstruct P {\n    x: Int\n}\n"), "there is no `@derive`: list the trait, `struct P: (Display)`");
}

#[test]
fn member_data_needs_a_modifier_that_reads_it() {
    let tagged = "@arg(x = a)\nstruct P: (Reader) {\n    a: Int\n}\n";
    let program = expand_with(tagged, &mut Fake::new()).unwrap();
    assert_eq!(methods_of(&program, "P"), ["read"]);
    let (message, span) = expand_with("@arg(x = a)\nstruct P: (Display) {\n    a: Int\n}\n", &mut Fake::new()).unwrap_err();
    assert_eq!(message, "`@arg` on a type is not read by any trait `P` lists");
    assert_eq!(span.line, 1);
    let (message, span) = expand_with("enum E: (Display) {\n    @arg(x = a)\n    A { a: Int }\n}\n", &mut Fake::new()).unwrap_err();
    assert_eq!(message, "`@arg` on a variant is not read by any trait `E` lists");
    assert_eq!(span.line, 2);
    let (message, _) = expand_with("@arg(x = a)\nstruct P {\n    a: Int\n}\n", &mut Fake::new()).unwrap_err();
    assert_eq!(message, "`@arg` on a type is not read by any trait `P` lists");
}

#[test]
fn a_modifier_lists_the_attributes_it_reads() {
    let program = parse("@modifier(reads = \"arg\")\npub fn show(d: Decl) -> Result<Additions, Diagnostic> {\n    return Err(Diagnostic.new(\"x\"))\n}\n");
    assert_eq!(program.modifier_marks[0].reads, ["arg"]);
    let none = parse("@modifier\npub fn show(d: Decl) -> Result<Additions, Diagnostic> {\n    return Err(Diagnostic.new(\"x\"))\n}\n");
    assert!(none.modifier_marks[0].reads.is_empty());
    assert_eq!(rejected("@modifier(reads = \"color\")\npub fn f(d: Decl) -> Result<Additions, Diagnostic> {\n}\n"), "`reads` names `color`, which is not an attribute a modifier can read");
    assert_eq!(rejected("@modifier(writes = \"arg\")\npub fn f(d: Decl) -> Result<Additions, Diagnostic> {\n}\n"), "`@modifier` takes `reads = \"…\"`, not `writes`");
}

#[test]
fn render_writes_the_generated_code_into_the_declaration() {
    let src = "struct P: (Debug) {\n    x: Int\n}\n";
    let program = expand_with(src, &mut Fake::new()).unwrap();
    let text = modifiers::render(src, &program.applied);
    let close = text.rfind('}').unwrap();
    let generated = text.find("// generated by Debug").expect("a generated block");
    assert!(generated < close, "{text}");
    assert!(text.contains("fn debug(self) -> String"), "{text}");
}

#[test]
fn the_registry_rejects_attributes_it_does_not_know_and_misplaced_ones() {
    let bad = |src: &str| {
        let tokens = Lexer::new(src).tokenize().unwrap();
        Parser::new(tokens).parse().unwrap_err().0
    };
    assert_eq!(bad("@derrive(Display)\nstruct P {\n    x: Int\n}\n"), "`@derrive` is not a known attribute");
    assert_eq!(bad("@generate(f)\nstruct P {\n}\n"), "`@generate` can only be used on a `trait`");
    assert_eq!(bad("@test(x)\nfn f() {}\n"), "`@test` takes no arguments");
    assert!(bad("struct P {\n    @color(x)\n    x: Int\n}\n").starts_with("`@color` cannot go on a field"));
    assert!(bad("struct P {\n    @test\n    x: Int\n}\n").starts_with("`@test` cannot go on a field"));
    assert_eq!(bad("@color(x)\nenum E {\n    A\n}\n"), "`@color` is not a known attribute");
    assert_eq!(bad("enum E {\n    @test\n    A\n}\n"), "`@test` cannot go on a variant");
}

const SHAPE: &str = "`@modifier` takes a `Decl` and returns `Result<Additions, Diagnostic>`";

fn rejected(src: &str) -> String {
    let tokens = Lexer::new(src).tokenize().unwrap();
    Parser::new(tokens).parse().unwrap_err().0
}

#[test]
fn a_modifier_function_is_recorded() {
    let program = parse("@modifier\npub fn show(d: meta.Decl) -> Result<meta.Additions, meta.Diagnostic> {\n    return Err(meta.Diagnostic.new(\"x\"))\n}\n");
    assert_eq!(program.modifier_marks.iter().map(|m| m.fn_name.as_str()).collect::<Vec<_>>(), ["show"]);
    let bare = parse("@modifier\npub fn show(d: Decl) -> Result<Additions, Diagnostic> {\n    return Err(Diagnostic.new(\"x\"))\n}\n");
    assert_eq!(bare.modifier_marks.len(), 1);
}

#[test]
fn a_modifier_must_be_pub_and_have_the_contract_signature() {
    assert_eq!(rejected("@modifier\nfn show(d: Decl) -> Result<Additions, Diagnostic> {\n}\n"), "`@modifier` needs a `pub fn`");
    assert_eq!(rejected("@modifier\npub fn show() -> Result<Additions, Diagnostic> {\n}\n"), SHAPE);
    assert_eq!(rejected("@modifier\npub fn show(d: Decl, e: Decl) -> Result<Additions, Diagnostic> {\n}\n"), SHAPE);
    assert_eq!(rejected("@modifier\npub fn show(d: Int) -> Result<Additions, Diagnostic> {\n}\n"), SHAPE);
    assert_eq!(rejected("@modifier\npub fn show(d: Decl) -> Additions {\n}\n"), SHAPE);
    assert_eq!(rejected("@modifier\npub fn show(d: Decl) -> Result<Additions, String> {\n}\n"), SHAPE);
}

#[test]
fn a_modifier_is_a_function_marker_only() {
    assert_eq!(rejected("@modifier\nstruct P {\n    x: Int\n}\n"), "`@modifier` can only be used on a `fn`");
    assert_eq!(rejected("@modifier(x)\npub fn f() {}\n"), "`@modifier` takes no arguments");
}

#[test]
fn a_trait_names_its_modifier_by_path_or_by_name() {
    let program = parse("@generate(std.derive.traits.display_modifier)\npub trait Display {\n    fn to_string(self) -> String\n}\n@generate(mine)\ntrait Mine {\n    fn mine(self) -> Int\n}\n");
    let generate: Vec<String> = program
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Trait(t) => t.attrs.iter().find(|a| a.name == "generate").map(|a| a.args[0].clone()),
            _ => None,
        })
        .collect();
    assert_eq!(generate, ["std.derive.traits.display_modifier", "mine"]);
}

#[test]
fn generate_is_for_traits_and_takes_one_path() {
    assert_eq!(rejected("@generate(f)\nfn g() {}\n"), "`@generate` can only be used on a `trait`");
    assert_eq!(rejected("@generate\ntrait T {\n}\n"), "`@generate` takes the path of one modifier function");
    assert_eq!(rejected("@generate(a, b)\ntrait T {\n}\n"), "`@generate` takes the path of one modifier function");
}

#[test]
fn a_declaration_is_written_as_json_for_stage_0() {
    let program = parse("/// a box\n/// of one value\n@arg(x = v)\npub class Box<T>: (Eq) {\n    /// the value\n    pub var v: T?\n}\n");
    let json = modifiers::declaration_json(&program, "Box").unwrap();
    assert!(json.starts_with("{\"kind\":\"class\",\"name\":\"Box\",\"pub\":true,\"doc\":\"a box\\nof one value\","), "{json}");
    assert!(json.contains("\"generics\":[{\"name\":\"T\",\"bounds\":[]}]"), "{json}");
    assert!(json.contains("\"traits\":[{\"kind\":\"name\",\"name\":\"Eq\",\"args\":[]}]"), "{json}");
    assert!(json.contains("\"pub\":true,\"var\":true,\"doc\":\"the value\",\"default\":\"\""), "{json}");
    assert!(json.contains("\"values\":[[\"x\",\"v\"]]"), "{json}");
    assert!(modifiers::declaration_json(&program, "Missing").is_none());
}

#[test]
fn a_field_default_is_written_as_source_text_for_stage_0() {
    let program = parse("class K {\n    n: Int = 8\n    d: Int = -1\n    s: String = \"a b\"\n    xs: List<String> = []\n    z: Int\n}\n");
    let json = modifiers::declaration_json(&program, "K").unwrap();
    assert!(json.contains("\"name\":\"n\",\"type\":{\"kind\":\"name\",\"name\":\"Int\",\"args\":[]},\"pub\":false,\"var\":false,\"doc\":\"\",\"default\":\"8\""), "{json}");
    assert!(json.contains("\"default\":\"-1\""), "{json}");
    assert!(json.contains("\"default\":\"\\\"a b\\\"\""), "{json}");
    assert!(json.contains("\"default\":\"[]\""), "{json}");
    assert!(json.contains("\"name\":\"z\",\"type\":{\"kind\":\"name\",\"name\":\"Int\",\"args\":[]},\"pub\":false,\"var\":false,\"doc\":\"\",\"default\":\"\""), "{json}");
}
