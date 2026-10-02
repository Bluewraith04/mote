use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use compiler::Compiler;
use isa::value::TypeRegistry;
use pkg::StandaloneBundler;

fn setup_temp_dir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mote_bundle_test_{}_{}", test_name, std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_bytecode_binary_serialization_roundtrip() {
    let source = r#"
fn add(a: Int, b: Int) -> Int {
    return a + b
}
let x = add(10, 20)
let y = add(30, 40)
return add(x, y)
"#;

    let original_compiled = Compiler::compile(source, "main.mote").unwrap();
    let bytes = original_compiled.to_bytes();
    assert!(!bytes.is_empty());
    assert_eq!(&bytes[0..6], b"MOTE\x01\x11");

    let restored_compiled = compiler::CompiledProgram::from_bytes(&bytes).unwrap();
    assert_eq!(restored_compiled.code_objects.len(), original_compiled.code_objects.len());
    assert_eq!(restored_compiled.type_descriptors.len(), original_compiled.type_descriptors.len());
    assert_eq!(restored_compiled.global_count, 2);
    assert_eq!(restored_compiled.global_count, original_compiled.global_count);

    let mut registry = TypeRegistry::new();
    for t in &restored_compiled.type_descriptors {
        let _ = registry.register(t.clone(), None);
    }
    let global_count = restored_compiled.global_count as usize;
    let mut rt = runtime::Runtime::with_type_registry(restored_compiled.code_objects, &registry);
    rt.set_global_count(global_count);
    let task = rt.run_entry().unwrap();
    assert_eq!(task.registers[0].as_int(), Some(100));
}

#[test]
fn test_standalone_payload_embedding_and_detection() {
    let temp = setup_temp_dir("embed_detect");
    let source_file = temp.join("app.mote");
    fs::write(&source_file, "fn calc() -> Int { return 777 }\nreturn calc()").unwrap();

    let compiled = Compiler::compile("fn calc() -> Int { return 777 }\nreturn calc()", "app.mote").unwrap();
    let payload = compiled.to_bytes();
    let payload_len = payload.len() as u64;

    let fake_exe = temp.join("fake_mote.exe");
    let mut f = File::create(&fake_exe).unwrap();
    f.write_all(b"MOCK_EXE_HEADER_BYTES_1234567890").unwrap();
    f.write_all(&payload).unwrap();
    f.write_all(&payload_len.to_le_bytes()).unwrap();
    f.write_all(StandaloneBundler::MAGIC_TRAILER).unwrap();
    f.flush().unwrap();

    let total_len = fake_exe.metadata().unwrap().len();
    assert!(total_len >= 23 + payload_len);

    let host_bytes = fs::read(&fake_exe).unwrap();
    assert_eq!(&host_bytes[host_bytes.len() - 15..], StandaloneBundler::MAGIC_TRAILER);

    fs::remove_dir_all(temp).ok();
}

fn fake_pe() -> Vec<u8> {
    let mut b = vec![0u8; 0x200];
    b[..2].copy_from_slice(b"MZ");
    b[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    b[0x40..0x44].copy_from_slice(b"PE\0\0");
    b[0x58..0x5A].copy_from_slice(&0x20bu16.to_le_bytes());
    b[0x58 + 108..0x58 + 112].copy_from_slice(&16u32.to_le_bytes());
    b
}

fn sign(mut exe: Vec<u8>) -> Vec<u8> {
    exe.resize(exe.len().next_multiple_of(8), 0);
    let offset = exe.len() as u32;
    exe.extend_from_slice(&[0xAB; 40]);
    let entry = 0x58 + 112 + 32;
    exe[entry..entry + 4].copy_from_slice(&offset.to_le_bytes());
    exe[entry + 4..entry + 8].copy_from_slice(&40u32.to_le_bytes());
    exe[0x58 + 64..0x58 + 68].copy_from_slice(&[1, 2, 3, 4]);
    exe
}

#[test]
fn a_signed_executable_keeps_its_payload_and_a_signed_host_is_stripped() {
    let temp = setup_temp_dir("pe_sign");
    let payload = Compiler::compile("fn calc() -> Int { return 777 }\nreturn calc()", "app.mote").unwrap().to_bytes();

    let host = sign(fake_pe());
    let built = StandaloneBundler::assemble(host, &payload);
    assert_eq!(built, StandaloneBundler::assemble(fake_pe(), &payload), "same bytes as from an unsigned host");
    assert_eq!(pkg::bundler::pe_certificate_table(&built), None);
    assert_eq!(built.len() % 8, 0);
    assert!(built.ends_with(StandaloneBundler::MAGIC_TRAILER));

    for (name, exe) in [("plain.exe", built.clone()), ("signed.exe", sign(built.clone()))] {
        let path = temp.join(name);
        fs::write(&path, &exe).unwrap();
        let program = StandaloneBundler::read_payload(&path).expect("payload found").unwrap();
        assert_eq!(program.to_bytes(), payload, "{name}");
    }

    let rebuilt = StandaloneBundler::assemble(sign(built.clone()), &payload);
    assert_eq!(rebuilt, built);

    fs::remove_dir_all(temp).ok();
}
