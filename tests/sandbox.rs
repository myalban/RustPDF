//! End-to-end check of protected view: the isolated worker starts, renders a page of a real PDF, and every
//! forbidden operation (reading a user file, writing, starting a program, network, executable memory) is blocked.
//! Linux is the exception for reading: files stay readable there, nothing can be written or sent.

use std::process::Command;

fn run_check(extra: &[&str]) -> (bool, String) {
    let exe = env!("CARGO_BIN_EXE_rustpdf");
    let out = Command::new(exe).arg("--sandbox-check").args(extra).current_dir(env!("CARGO_MANIFEST_DIR")).output().expect("the application starts");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn the_worker_renders_a_page_and_is_isolated() {
    let (ok, text) = run_check(&["samples/demo.pdf", "--selftest"]);
    println!("{text}");
    assert!(text.contains("rendered page 1"), "the worker must render the page:\n{text}");
    assert!(text.contains("every forbidden operation was blocked"), "the worker must be isolated:\n{text}");
    assert!(ok, "the check must succeed:\n{text}");
}

#[test]
fn hostile_or_broken_files_never_take_the_application_down() {
    let dir = std::env::temp_dir().join("rustpdf_hostile_files");
    std::fs::create_dir_all(&dir).unwrap();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("garbage.pdf", (0..5000u32).map(|i| (i * 31 % 251) as u8).collect()),
        ("header_only.pdf", b"%PDF-1.7\n".to_vec()),
        ("truncated.pdf", std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/samples/demo.pdf")).unwrap()[..900].to_vec()),
        ("huge_page.pdf", b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 99999999 99999999]>>endobj\ntrailer<</Root 1 0 R/Size 4>>\n%%EOF".to_vec()),
        ("page_tree_loop.pdf", b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[2 0 R]/Count 1>>endobj\ntrailer<</Root 1 0 R/Size 3>>\n%%EOF".to_vec()),
    ];
    for (name, data) in cases {
        let path = dir.join(name);
        std::fs::write(&path, data).unwrap();
        let (_, text) = run_check(&[path.to_str().unwrap()]);
        println!("--- {name}\n{text}");
        // whatever the outcome, the check itself must not hang or crash: it prints the result of every step
        assert!(text.contains("ready, isolation"), "{name}: the worker must start:\n{text}");
    }
}

/// Random byte flips of a real document: the worker may refuse or even crash on some of them, the check itself must
/// always finish and report. Run with `cargo test --test sandbox mutated -- --ignored --nocapture`.
#[test]
#[ignore]
fn mutated_documents_are_contained() {
    let original = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/samples/demo.pdf")).unwrap();
    let dir = std::env::temp_dir().join("rustpdf_mutations");
    std::fs::create_dir_all(&dir).unwrap();
    let mut seed = 0x2545F4914F6CDD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let (mut opened, mut refused, mut crashed) = (0, 0, 0);
    for i in 0..150 {
        let mut data = original.clone();
        for _ in 0..(1 + next() % 40) {
            let pos = (next() as usize) % data.len();
            data[pos] = next() as u8;
        }
        if i % 10 == 0 {
            data.truncate((next() as usize) % data.len());
        }
        let path = dir.join(format!("m{i}.pdf"));
        std::fs::write(&path, &data).unwrap();
        let (_, text) = run_check(&[path.to_str().unwrap()]);
        assert!(text.contains("ready, isolation"), "mutation {i}: the worker must start:\n{text}");
        if text.contains("rendered page 1") {
            opened += 1;
        } else if text.contains("stream ended") || text.contains("no answer") {
            crashed += 1;
            println!("mutation {i} made the worker stop:\n{text}");
        } else {
            refused += 1;
        }
    }
    println!("rendered: {opened}, refused: {refused}, worker stopped (contained): {crashed}");
}
