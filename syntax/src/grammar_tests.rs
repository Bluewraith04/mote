//! Every rule of `Syntax.md` has an accepted and a rejected input in `grammar.cases`, with the result each gives.

use std::collections::BTreeSet;

use mote_base::{DiagnosticSink, Interner, Severity, SourceMap};

use crate::ast::dump::Dump;
use crate::lexer::{lex, CommentKind};
use crate::parser::parse;
use crate::token::TokenKind;

const SYNTAX: &str = include_str!("../Syntax.md");
const CASES: &str = include_str!("../grammar.cases");
const SEPARATOR: &str = " ‖ ";

/// A rule of `Syntax.md`.
struct Rule {
    /// The left side, without quotes, and `#n` after the first of a repeated left side.
    key: String,
    /// The words of the left side.
    left: Vec<String>,
    /// In the token section.
    token: bool,
}

/// A line of `grammar.cases`.
struct Case {
    line: usize,
    rule: String,
    verb: String,
    source: String,
    result: String,
}

fn rules() -> Vec<Rule> {
    let mut found: Vec<Rule> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut token = true;
    for line in SYNTAX.lines() {
        if line.starts_with("## ") {
            token = line.starts_with("## Tokens");
        }
        let Some(rest) = line.strip_prefix("- ") else { continue };
        let Some((left, _)) = rest.split_once(" ↔ ") else { continue };
        let name = left.replace('`', "");
        let repeats = seen.iter().filter(|known| **known == name).count();
        seen.push(name.clone());
        found.push(Rule { key: if repeats == 0 { name.clone() } else { format!("{name}#{}", repeats + 1) }, left: name.split(' ').map(str::to_string).collect(), token });
    }
    // The first of a repeated left side is also reachable as `name#1`.
    for rule in &mut found {
        if seen.iter().filter(|known| **known == rule.left.join(" ")).count() > 1 && !rule.key.contains('#') {
            rule.key = format!("{}#1", rule.key);
        }
    }
    found
}

fn cases() -> Vec<Case> {
    let mut found = Vec::new();
    for (number, line) in CASES.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(4, SEPARATOR).collect();
        assert_eq!(parts.len(), 4, "grammar.cases:{}: not `rule{SEPARATOR}verb{SEPARATOR}source{SEPARATOR}result`", number + 1);
        found.push(Case { line: number + 1, rule: parts[0].to_string(), verb: parts[1].to_string(), source: unescape(parts[2]), result: parts[3].to_string() });
    }
    found
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The token and comment names of a source, in order, ending in `Eof`, as the name and what shows of the token, and the errors.
fn lexed(source: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut map = SourceMap::new();
    let id = map.add_file("case", source);
    let mut names = Interner::new();
    let mut sink = DiagnosticSink::new();
    let lexed = lex(map.file(id), &mut names, &mut sink);
    let mut items: Vec<(u32, String, String)> = Vec::new();
    for token in &lexed.tokens {
        let head = format!("{:?}", token.kind).split('(').next().unwrap_or_default().to_string();
        let shown = match token.kind {
            TokenKind::Int(literal) => format!("Int({:?} {} {})", literal.radix, literal.value, literal.suffix.map_or("none".to_string(), |suffix| format!("{suffix:?}"))),
            TokenKind::Float(literal) => format!("Float({} {})", literal.value, literal.suffix.map_or("none".to_string(), |suffix| format!("{suffix:?}"))),
            _ => head.clone(),
        };
        items.push((token.span.start, head, shown));
    }
    for comment in &lexed.comments {
        let name = match comment.kind {
            CommentKind::Line => "LineComment",
            CommentKind::Doc => "DocComment",
            CommentKind::Block => "BlockComment",
        };
        items.push((comment.span.start, name.to_string(), name.to_string()));
    }
    items.sort_by_key(|(start, head, _)| (*start, head == "Eof"));
    let errors = sink.iter().filter(|d| d.severity == Severity::Error).map(|d| d.message.clone()).collect();
    (items.into_iter().map(|(_, head, shown)| (head, shown)).collect(), errors)
}

/// The tree of a source as one line, and the errors.
fn parsed(source: &str) -> (String, Vec<String>, Vec<String>) {
    let mut map = SourceMap::new();
    let id = map.add_file("case", source);
    let mut names = Interner::new();
    let mut sink = DiagnosticSink::new();
    let lexed = lex(map.file(id), &mut names, &mut sink);
    let ast = parse(map.file(id), &lexed, &mut names, &mut sink);
    let errors = sink.iter().filter(|d| d.severity == Severity::Error).map(|d| d.message.clone()).collect();
    let warnings = sink.iter().filter(|d| d.severity == Severity::Warning).map(|d| d.message.clone()).collect();
    (Dump { ast: &ast, names: &names }.root().replace('\n', " ; "), errors, warnings)
}

fn contains(names: &[(String, String)], left: &[String]) -> bool {
    names.windows(left.len()).any(|window| window.iter().zip(left).all(|((head, _), word)| head == word))
}

/// What a case gives, or why it does not do what its verb says.
fn evaluate(rule: &Rule, case: &Case) -> Result<String, String> {
    let recovery = matches!(rule.key.as_str(), "ErrorItem" | "ErrorStmt");
    if rule.token {
        let (names, errors) = lexed(&case.source);
        let only_eof = names.len() == 1 && names[0].0 == "Eof";
        let radix = match rule.key.as_str() {
            "Int#1" => "Int(Hexadecimal ",
            "Int#2" => "Int(Binary ",
            "Int#3" => "Int(Octal ",
            "Int#4" => "Int(Decimal ",
            _ => "",
        };
        let right_radix = names.iter().any(|(_, shown)| shown.starts_with(radix));
        let shows = errors.is_empty() && right_radix && if rule.left == ["Whitespace"] { !case.source.is_empty() && only_eof } else { contains(&names, &rule.left) };
        let result = if errors.is_empty() { names.iter().map(|(_, shown)| shown.as_str()).collect::<Vec<_>>().join(" ") } else { format!("error: {}", errors[0]) };
        return match (case.verb.as_str(), shows) {
            ("accept", true) | ("reject", false) => Ok(result),
            ("accept", false) => Err(format!("is marked accept and gives `{result}`")),
            ("reject", true) => Err(format!("is marked reject and gives `{result}`")),
            (other, _) => Err(format!("has the verb `{other}`; a token rule takes accept and reject")),
        };
    }
    let (tree, errors, warnings) = parsed(&case.source);
    match case.verb.as_str() {
        "warn" if errors.is_empty() && !warnings.is_empty() => Ok(format!("warning: {}", warnings[0])),
        "warn" => Err(format!("is marked warn and gives `{tree}` with {} errors", errors.len())),
        "accept" if errors.is_empty() => Ok(tree),
        "accept" => Err(format!("is marked accept and gives the error `{}`", errors[0])),
        "reject" if !errors.is_empty() => Ok(format!("error: {}", errors[0])),
        "reject" => Err(format!("is marked reject and gives `{tree}`")),
        "recover" if recovery && !errors.is_empty() => Ok(format!("{} => {tree}", errors.join("; "))),
        "recover" if recovery => Err("is marked recover and gives no error".to_string()),
        other => Err(format!("has the verb `{other}`; a node rule takes accept, reject and warn, and an error node recover")),
    }
}

#[test]
fn grammar_the_rules_of_the_document_are_found_with_distinct_keys() {
    let rules = rules();
    let keys: BTreeSet<&str> = rules.iter().map(|rule| rule.key.as_str()).collect();
    assert_eq!(keys.len(), rules.len(), "two rules have the same key");
    assert!(rules.len() > 200, "{} rules", rules.len());
    assert!(keys.contains("LineComment#3") && keys.contains("Int#4") && keys.contains("Import#2") && keys.contains("Let"));
}

#[test]
fn grammar_every_case_names_a_rule() {
    let keys: BTreeSet<String> = rules().into_iter().map(|rule| rule.key).collect();
    let unknown: Vec<String> = cases().into_iter().filter(|case| !keys.contains(&case.rule)).map(|case| format!("grammar.cases:{}: no rule is called `{}`", case.line, case.rule)).collect();
    assert!(unknown.is_empty(), "{}", unknown.join("\n"));
}

/// The rules that lack an accepted or a rejected input among the cases.
fn uncovered(rules: &[Rule], cases: &[Case]) -> Vec<String> {
    let mut missing = Vec::new();
    for rule in rules {
        let verbs: BTreeSet<&str> = cases.iter().filter(|case| case.rule == rule.key).map(|case| case.verb.as_str()).collect();
        let recovery = matches!(rule.key.as_str(), "ErrorItem" | "ErrorStmt");
        if recovery {
            if !verbs.contains("recover") || !verbs.contains("reject") {
                missing.push(format!("`{}` needs a recover and a reject case", rule.key));
            }
        } else {
            if !verbs.contains("accept") {
                missing.push(format!("`{}` has no accepted input", rule.key));
            }
            if !verbs.contains("reject") && !verbs.contains("warn") {
                missing.push(format!("`{}` has no rejected input", rule.key));
            }
        }
    }
    missing
}

#[test]
fn grammar_every_rule_has_an_accepted_and_a_rejected_input() {
    let missing = uncovered(&rules(), &cases());
    assert!(missing.is_empty(), "{} rules lack cases:\n{}", missing.len(), missing.join("\n"));
}

#[test]
fn grammar_a_rule_without_both_cases_is_reported() {
    let rule = |key: &str| Rule { key: key.to_string(), left: vec![key.to_string()], token: false };
    let case = |rule: &str, verb: &str| Case { line: 1, rule: rule.to_string(), verb: verb.to_string(), source: String::new(), result: String::new() };
    let rules = [rule("A"), rule("B"), rule("C"), rule("ErrorItem")];
    let cases = [case("A", "accept"), case("A", "reject"), case("B", "accept"), case("C", "warn"), case("ErrorItem", "recover")];
    assert_eq!(uncovered(&rules, &cases), ["`B` has no rejected input", "`C` has no accepted input", "`ErrorItem` needs a recover and a reject case"]);
}

#[test]
fn grammar_every_case_gives_the_result_it_records() {
    let rules = rules();
    let bless = std::env::var_os("MOTE_BLESS").is_some();
    let mut problems = Vec::new();
    let mut results = Vec::new();
    for case in cases() {
        let Some(rule) = rules.iter().find(|rule| rule.key == case.rule) else { continue };
        match evaluate(rule, &case) {
            Err(problem) => problems.push(format!("grammar.cases:{}: `{}` {} {}", case.line, case.rule, case.source.escape_debug(), problem)),
            Ok(result) => {
                if result != case.result && !bless {
                    problems.push(format!("grammar.cases:{}: `{}` {} gives `{}` and the file says `{}`", case.line, case.rule, case.source.escape_debug(), result, case.result));
                }
                results.push((case.line, result));
            }
        }
    }
    if bless && problems.is_empty() {
        let mut out = String::new();
        for (number, line) in CASES.lines().enumerate() {
            match results.iter().find(|(at, _)| *at == number + 1) {
                Some((_, result)) => {
                    let head: Vec<&str> = line.splitn(4, SEPARATOR).collect();
                    out.push_str(&format!("{}{SEPARATOR}{}{SEPARATOR}{}{SEPARATOR}{result}\n", head[0], head[1], head[2]));
                }
                None => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/grammar.cases"), out).expect("a written file");
        return;
    }
    assert!(problems.is_empty(), "{} problems (MOTE_BLESS=1 records results):\n{}", problems.len(), problems.join("\n"));
}
