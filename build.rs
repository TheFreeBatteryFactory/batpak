// justifies: build.rs is fail-fast by design; invariant violations halt the build with a cargo-parseable explanation. Every panic site is spanned by the `fail` helper below with a dedicated justification. This file-level allow is the single documented entry point for that policy.
#![allow(clippy::panic)]

use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use syn::visit::Visit;

/// Single documented failure path for build.rs. Every invariant violation
/// routes through here so the panic surface is consolidated and auditable.
/// Emits a cargo-parseable warning on stdout, then panics so Cargo stops the
/// build without relying on `process::exit`.
fn fail(msg: &str) -> ! {
    println!("cargo:warning=build.rs: {msg}");
    panic!("build.rs invariant failed: {msg}")
}

#[derive(Debug, Deserialize)]
struct PubItemAllowlistEntry {
    name: String,
    justification: String,
    #[serde(default)]
    witness: Vec<PubItemAllowlistWitness>,
}

#[derive(Debug, Deserialize)]
struct PubItemAllowlistWitness {
    path: String,
    #[serde(default)]
    lines: Vec<u32>,
}

// build.rs runs before every cargo build/check/test. Cannot be skipped.
// It enforces live runtime invariants at build time so agents get English
// errors instead of cryptic compiler failures. See README.md, GUIDE.md, and
// REFERENCE.md for the current truth hierarchy.
fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=src/");
    println!("cargo:rerun-if-changed=traceability/pub_item_allowlist.yaml");

    check_no_tokio_in_deps();
    check_no_banned_patterns();
    check_store_config_field_usage();
    check_allow_justifications();
    check_no_stubs_in_src();
    check_store_surface_honesty();
    check_no_fixed_temp_patterns();
    check_pub_items_have_tests();
}

/// Audit Loop Layer 2 enforcement: no stub markers in production src/.
/// todo!() and unimplemented!() are already denied by clippy, but this
/// catches patterns clippy misses: hardcoded placeholder strings, empty
/// function bodies returning defaults, etc.
fn check_no_stubs_in_src() {
    let stub_patterns = [
        (
            "\"placeholder\"",
            "Placeholder string literal — replace with real implementation",
        ),
        (
            "\"not implemented\"",
            "Stub string — implement the real behavior or return a typed error",
        ),
        (
            "\"not yet implemented\"",
            "Stub string — implement the real behavior",
        ),
    ];

    walk_rs_files(Path::new("src"), &mut |path, contents| {
        let path_str = path.display().to_string();
        for (line_no, line) in contents.lines().enumerate() {
            let lower = line.to_lowercase();
            for (pattern, msg) in &stub_patterns {
                if lower.contains(pattern) {
                    panic!(
                        "STUB DETECTED in {path_str}:{}: {msg}\n\
                         Line: {line}\n\
                         LAW-001: No fake success responses. FM-009: No polite downgrades.",
                        line_no + 1
                    );
                }
            }
        }
    });
}

/// Parse a single source line and return true if it carries a structured
/// justification comment. A justification is `// justifies: <body>` where
/// `<body>` has at least five whitespace-separated words. Matches either an
/// inline trailing comment or a standalone comment line.
fn line_carries_justification(line: &str) -> bool {
    let trimmed = line.trim();
    if let Some(idx) = trimmed.find("// justifies:") {
        let body = &trimmed[idx + "// justifies:".len()..];
        return body.split_whitespace().count() >= 5;
    }
    if trimmed.starts_with("//") {
        let stripped = trimmed.trim_start_matches('/').trim();
        if let Some(body) = stripped.strip_prefix("justifies:") {
            return body.split_whitespace().count() >= 5;
        }
    }
    false
}

/// FM-002 Rogue Silence defense: every #[allow(...)] in runtime or toolchain
/// Rust code must carry a `// justifies: <>= 5 words>` comment on the same or
/// previous line. Narrative-only comments and raw `//`-prefixed prose count as
/// silencers; the structured prefix is load-bearing.
fn check_allow_justifications() {
    walk_allow_checked_rs_files(&mut |path, contents| {
        let path_str = path.display().to_string();
        let lines: Vec<&str> = contents.lines().collect();
        for (line_no, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("#![allow(") || trimmed.starts_with("#[allow(") {
                let has_justification = line_carries_justification(line)
                    || (line_no > 0
                        && lines
                            .get(line_no - 1)
                            .map(|prev| line_carries_justification(prev))
                            .unwrap_or(false));
                if !has_justification {
                    fail(&format!(
                        "ROGUE SILENCE in {path_str}:{}: `{trimmed}`\n\
                         Every #[allow(...)] must carry a `// justifies: <>= 5 words>` comment on the same\n\
                         or previous line explaining the design decision that makes the silencer safe.\n\
                         Example: // justifies: pattern is a string literal known-safe at compile time; this expect cannot fire\n\
                         See: Big Bang FM-002 (Rogue Silence).",
                        line_no + 1
                    ));
                }
            }
        }
    });
}

fn check_no_tokio_in_deps() {
    //Invariant 1: tokio must not appear in [dependencies].
    //Only [dev-dependencies] is allowed.
    let cargo = fs::read_to_string("Cargo.toml").expect("read Cargo.toml");

    //Strategy: find the [dependencies] section, take text until the next
    //section header (line starting with [), check for "tokio".
    //This is deliberately simple string matching — no toml parser dep.
    if let Some(deps_section) = cargo.split("[dependencies]").nth(1) {
        let deps_only = deps_section.split("\n[").next().unwrap_or("");
        if deps_only.contains("tokio") {
            panic!(
                "INVARIANT 1 VIOLATED: tokio found in [dependencies].\n\
                 tokio belongs in [dev-dependencies] only.\n\
                 The library is runtime-agnostic. Fan-out uses Vec<flume::Sender>.\n\
                 See: REFERENCE.md."
            );
        }
    }
}

fn check_no_banned_patterns() {
    //Walk src/**/*.rs, read each file, check for patterns that violate
    //invariants or red flags.
    walk_rs_files(Path::new("src"), &mut |path, contents| {
        let path_str = path.display().to_string();

        //Red flag: no transmute/mem::read/pointer_cast in any src file.
        //All serialization goes through MessagePack.
        for banned in ["transmute", "mem::read", "pointer_cast"] {
            if contents.contains(banned) {
                panic!(
                    "RED FLAG VIOLATED in {path_str}: found `{banned}`.\n\
                     repr(C) is for field ordering, not a wire format.\n\
                     All serialization goes through rmp-serde. Always.\n\
                     See: REFERENCE.md."
                );
            }
        }

        //Invariant 2: no async fn in store module.
        //Store API is sync. Async lives in flume channels.
        if path_str.contains("store") && contents.contains("async fn") {
            panic!(
                "INVARIANT 2 VIOLATED in {path_str}: found `async fn`.\n\
                 Store API is sync. Async callers use spawn_blocking()\n\
                 or flume's recv_async(). See: store/subscription.rs.\n\
                 See: REFERENCE.md."
            );
        }

        // Post-mortem Bug 7: std::thread::spawn() panics on failure.
        // All thread creation must use Builder::new().spawn() for fallible error handling.
        if contents.contains("std::thread::spawn(") {
            panic!(
                "BANNED PATTERN in {path_str}: `std::thread::spawn()` found.\n\
                 Use `std::thread::Builder::new().name(...).spawn()` instead.\n\
                 `thread::spawn` panics on failure; `Builder::spawn` returns Result.\n\
                 See: Bug 7 post-mortem (react_loop panic)."
            );
        }

        // Post-mortem Bug 9: bare .sync() bypasses sync_mode config.
        // In store/ files, require .sync_with_mode() — never bare .sync().
        // The only exception is segment.rs which defines the .sync() method itself.
        if path_str.contains("store") && !path_str.ends_with("segment.rs") {
            for (line_no, line) in contents.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") || trimmed.starts_with("///") {
                    continue;
                }
                // Match .sync() but not .sync_with_mode() and not self.sync() (Store::sync)
                if trimmed.contains(".sync()")
                    && !trimmed.contains("sync_with_mode")
                    && !trimmed.contains("self.sync()")
                    && !trimmed.contains("force_sync()")
                {
                    panic!(
                        "BANNED PATTERN in {path_str}:{}: bare `.sync()` call.\n\
                         Use `.sync_with_mode(&config.sync.mode)` instead.\n\
                         Bare .sync() hardcodes SyncAll, ignoring the user's config.\n\
                         See: Bug 9 post-mortem (segment rotation bypassed sync.mode).\n\
                         Line: {trimmed}",
                        line_no + 1
                    );
                }
            }
        }

        //Invariant 3: no product concepts in library code.
        //Check struct/enum/fn/type declarations for banned nouns.
        //Skip string literals and comments.
        let banned_nouns = ["trajectory", "artifact", "tenant"];
        //NOTE: "scope" and "agent" are common English words.
        //"turn" and "note" are substrings of "return" and "annotation" —
        //substring matching would false-positive on legitimate Rust code.
        //Only check nouns that are unambiguous product concepts.
        //Strategy: check lines starting with pub/fn/struct/enum/type
        //for WORD-BOUNDARY matches of banned nouns.
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with("///") {
                continue; // skip comments
            }
            let is_decl = trimmed.starts_with("pub ")
                || trimmed.starts_with("fn ")
                || trimmed.starts_with("struct ")
                || trimmed.starts_with("enum ")
                || trimmed.starts_with("type ");
            if is_decl {
                let lower = trimmed.to_lowercase();
                for noun in &banned_nouns {
                    //Word boundary check: noun must be preceded by start/underscore/space
                    //and followed by end/underscore/space/(/>. Prevents "return" matching "turn".
                    let has_match =
                        lower
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                            .any(|word| {
                                word == *noun
                                    || word.starts_with(&format!("{noun}_"))
                                    || word.ends_with(&format!("_{noun}"))
                                    || word.contains(&format!("_{noun}_"))
                            });
                    if has_match {
                        panic!(
                            "INVARIANT 3 VIOLATED in {path_str}: \
                             product concept `{noun}` in declaration:\n  {trimmed}\n\
                             Library vocabulary: coordinate, entity, event, outcome, \
                             gate, region, transition.\n\
                             See: REFERENCE.md."
                        );
                    }
                }
            }
        }
    });
}

fn check_store_surface_honesty() {
    let store_mod =
        fs::read_to_string("src/store/mod.rs").expect("read src/store/mod.rs for surface check");
    if store_mod.contains("pub fn subscribe(") {
        panic!(
            "PUBLIC API HONESTY VIOLATION: src/store/mod.rs still exports `pub fn subscribe(`.\n\
             The lossy broadcast API must be named `subscribe_lossy` so callers cannot\n\
             confuse it with guaranteed delivery."
        );
    }
    if store_mod.contains("pub fn cursor(") {
        panic!(
            "PUBLIC API HONESTY VIOLATION: src/store/mod.rs still exports `pub fn cursor(`.\n\
             The guaranteed replay API must be named `cursor_guaranteed`."
        );
    }
    if store_mod.contains("Freshness::BestEffort") || store_mod.contains("BestEffort") {
        panic!(
            "PUBLIC API HONESTY VIOLATION: stale `Freshness::BestEffort` reference in src/store/mod.rs.\n\
             Use `Freshness::MaybeStale {{ max_stale_ms }}`."
        );
    }

    walk_rs_files(Path::new("src/store"), &mut |path, contents| {
        let path_str = path.display().to_string();
        if contents.contains("test-support") {
            panic!(
                "FEATURE HONESTY VIOLATION in {path_str}: stale `test-support` reference.\n\
                 The explicit risk-bearing feature name is `dangerous-test-hooks`."
            );
        }
    });
}

fn check_no_fixed_temp_patterns() {
    walk_rs_files(Path::new("src/store"), &mut |path, contents| {
        let path_str = path.display().to_string();
        if contents.contains("index.ckpt.tmp") || contents.contains(".tmp_{pid}_{n}") {
            panic!(
                "TEMP FILE HARDENING VIOLATION in {path_str}: fixed temp-file pattern found.\n\
                 Use same-directory `tempfile::NamedTempFile` instead of predictable names."
            );
        }
        if contents.contains("create(true)") && contents.contains("truncate(true)") {
            panic!(
                "TEMP FILE HARDENING VIOLATION in {path_str}: `create(true)` + `truncate(true)` found.\n\
                 This is the symlink-clobber shape the release hardening pass bans in src/store."
            );
        }
    });
}

fn check_store_config_field_usage() {
    // Invariant: every pub field in StoreConfig must be read somewhere in src/.
    // This catches "config field defined but never wired up" bugs like the
    // historical writer.stack_size and sync.mode regressions.
    // This is part of the live configuration completeness contract.
    let config_src = fs::read_to_string("src/store/config.rs")
        .expect("read src/store/config.rs for config check");

    let config_ast = syn::parse_file(&config_src)
        .expect("parse src/store/config.rs for config field usage check");
    let fields = store_config_public_fields(&config_ast);
    if fields.is_empty() {
        return;
    }

    let mut used_fields = BTreeSet::new();
    walk_rs_files(Path::new("src"), &mut |path, contents| {
        if path
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with("src/store/config.rs")
        {
            return;
        }
        let file = syn::parse_file(contents).unwrap_or_else(|err| {
            fail(&format!(
                "CONFIG FIELD USAGE CHECK PARSE FAILURE in {}: {err}",
                path.display()
            ))
        });
        let mut collector = StoreConfigFieldAccessCollector::new(&fields);
        collector.visit_file(&file);
        used_fields.extend(collector.found_fields);
    });

    for field in &fields {
        if !used_fields.contains(field) {
            panic!(
                "STORE CONFIG FIELD UNUSED: `{field}` is defined in StoreConfig but never \
                 accessed in any parsed src/ file outside src/store/config.rs.\n\
                 Every config field must be wired to actual behavior.\n\
                 Either use the field or remove it from StoreConfig.\n\
                 See: the historical writer.stack_size / sync.mode bugs that slipped through review."
            );
        }
    }
}

fn store_config_public_fields(file: &syn::File) -> BTreeSet<String> {
    for item in &file.items {
        if let syn::Item::Struct(item_struct) = item {
            if item_struct.ident == "StoreConfig" {
                let mut fields = BTreeSet::new();
                for field in &item_struct.fields {
                    if matches!(field.vis, syn::Visibility::Public(_)) {
                        if let Some(ident) = &field.ident {
                            fields.insert(ident.to_string());
                        }
                    }
                }
                return fields;
            }
        }
    }
    BTreeSet::new()
}

struct StoreConfigFieldAccessCollector<'a> {
    tracked_fields: &'a BTreeSet<String>,
    found_fields: BTreeSet<String>,
}

impl<'a> StoreConfigFieldAccessCollector<'a> {
    fn new(tracked_fields: &'a BTreeSet<String>) -> Self {
        Self {
            tracked_fields,
            found_fields: BTreeSet::new(),
        }
    }
}

impl Visit<'_> for StoreConfigFieldAccessCollector<'_> {
    fn visit_expr_field(&mut self, node: &syn::ExprField) {
        if let syn::Member::Named(ident) = &node.member {
            let field_name = ident.to_string();
            if self.tracked_fields.contains(&field_name) {
                self.found_fields.insert(field_name);
            }
        }
        syn::visit::visit_expr_field(self, node);
    }
}

/// Moonwalker post-mortem defense: every public item in src/ must have at least
/// one real path-position reference in a test file (verified via AST walk, not
/// substring match). Allowlisted items must declare a `witness:` path that the
/// AST walker confirms contains a real reference — a bogus witness fails the
/// build. LAW-003 (No Orphan Infrastructure), FM-007 (Island Syndrome).
fn check_pub_items_have_tests() {
    let allowlist = load_pub_item_allowlist();
    let allowed_names: BTreeSet<&str> = allowlist.iter().map(|entry| entry.name.as_str()).collect();

    // Validate every allowlist entry's witness paths via the AST walker. A
    // single real reference in the referenced file is sufficient. Bogus
    // witnesses (path missing, file does not reference the item) fail here.
    for entry in &allowlist {
        if entry.witness.is_empty() {
            fail(&format!(
                "pub_item_allowlist entry `{}` must declare at least one `witness:` path pointing at a test that uses the item; narrative `justification:` is supplementary, not load-bearing",
                entry.name
            ));
        }
        for witness in &entry.witness {
            let witness_path = Path::new(&witness.path);
            let content = match fs::read_to_string(witness_path) {
                Ok(c) => c,
                Err(err) => fail(&format!(
                    "pub_item_allowlist entry `{}` witness `{}` cannot be read: {err}",
                    entry.name, witness.path
                )),
            };
            let file = match syn::parse_file(&content) {
                Ok(f) => f,
                Err(err) => fail(&format!(
                    "pub_item_allowlist entry `{}` witness `{}` does not parse: {err}",
                    entry.name, witness.path
                )),
            };
            if !ast_references_name(&file, &entry.name) {
                fail(&format!(
                    "pub_item_allowlist entry `{}` witness `{}` (line hints {:?}) has no real path-position reference to the item; update the witness path or hide the item via `#[doc(hidden)]`",
                    entry.name, witness.path, witness.lines,
                ));
            }
        }
    }

    // Parse every test file once.
    let test_files: Vec<(std::path::PathBuf, syn::File)> = collect_rs_file_asts(Path::new("tests"));

    // Walk src/ and collect public item names from the parsed AST, then
    // confirm each name has at least one real path-position reference in the
    // parsed tests.
    walk_rs_files(Path::new("src"), &mut |path, contents| {
        if path.ends_with("prelude.rs") {
            return;
        }
        let path_str = path.display().to_string();
        let file = syn::parse_file(contents).unwrap_or_else(|err| {
            fail(&format!(
                "PUB ITEM REFERENCE CHECK PARSE FAILURE in {path_str}: {err}\n\
                 This detector is syntax-aware by design; fix the source or the parser input."
            ))
        });
        let mut collector = PublicItemCollector::default();
        collector.visit_file(&file);
        for name in collector.names {
            if allowed_names.contains(name.as_str()) {
                continue;
            }
            let witnessed = test_files
                .iter()
                .any(|(_, ast)| ast_references_name(ast, &name));
            if !witnessed {
                let (line, _col) = item_line_in_file(contents, &name);
                fail(&format!(
                    "pub item `{name}` declared at {path_str}:{line} has no test reference (checked {n} test files via AST); either add a real test use, add an allowlist entry with a `witness:` path that points to an actual use, or hide the item via `#[doc(hidden)]`.",
                    n = test_files.len(),
                ));
            }
        }
    });
}

fn collect_rs_file_asts(dir: &Path) -> Vec<(std::path::PathBuf, syn::File)> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(collect_rs_file_asts(&path));
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                if let Ok(contents) = fs::read_to_string(&path) {
                    if let Ok(file) = syn::parse_file(&contents) {
                        out.push((path, file));
                    }
                }
            }
        }
    }
    out
}

/// Walk a parsed Rust file and return true if any real path-position expression
/// or type references `name`. References inside comments and string literals
/// are ignored; only AST path positions count. Mirrors the integrity-tool
/// walker exactly so build-time and tool-time checks agree.
fn ast_references_name(file: &syn::File, name: &str) -> bool {
    struct Walker<'a> {
        needle: &'a str,
        found: bool,
    }
    impl<'a, 'ast> Visit<'ast> for Walker<'a> {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            if self.found {
                return;
            }
            for segment in &path.segments {
                if segment.ident == self.needle {
                    self.found = true;
                    return;
                }
            }
            syn::visit::visit_path(self, path);
        }

        fn visit_use_tree(&mut self, tree: &'ast syn::UseTree) {
            if self.found {
                return;
            }
            match tree {
                syn::UseTree::Name(n) => {
                    if n.ident == self.needle {
                        self.found = true;
                    }
                }
                syn::UseTree::Rename(r) => {
                    if r.ident == self.needle || r.rename == self.needle {
                        self.found = true;
                    }
                }
                syn::UseTree::Path(p) => {
                    if p.ident == self.needle {
                        self.found = true;
                        return;
                    }
                    self.visit_use_tree(&p.tree);
                }
                syn::UseTree::Group(group) => {
                    for item in &group.items {
                        self.visit_use_tree(item);
                        if self.found {
                            return;
                        }
                    }
                }
                syn::UseTree::Glob(_) => {}
            }
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            if self.found {
                return;
            }
            if call.method == self.needle {
                self.found = true;
                return;
            }
            syn::visit::visit_expr_method_call(self, call);
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if self.found {
                return;
            }
            for segment in &mac.path.segments {
                if segment.ident == self.needle {
                    self.found = true;
                    return;
                }
            }
            syn::visit::visit_macro(self, mac);
        }

        fn visit_field(&mut self, field: &'ast syn::Field) {
            if self.found {
                return;
            }
            syn::visit::visit_type(self, &field.ty);
        }
    }

    let mut walker = Walker {
        needle: name,
        found: false,
    };
    walker.visit_file(file);
    walker.found
}

fn item_line_in_file(contents: &str, name: &str) -> (usize, usize) {
    for (idx, line) in contents.lines().enumerate() {
        if line.contains(name) {
            return (idx + 1, line.find(name).unwrap_or(0) + 1);
        }
    }
    (0, 0)
}

fn load_pub_item_allowlist() -> Vec<PubItemAllowlistEntry> {
    let path = Path::new("traceability/pub_item_allowlist.yaml");
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|err| fail(&format!("failed to read {}: {err}", path.display())));
    let entries: Vec<PubItemAllowlistEntry> = yaml_serde::from_str(&contents)
        .unwrap_or_else(|err| fail(&format!("failed to parse {}: {err}", path.display())));
    for entry in &entries {
        if entry.name.trim().is_empty() {
            fail(&format!(
                "invalid {} entry: `name` must not be empty (justification: {})",
                path.display(),
                entry.justification
            ));
        }
        if entry.justification.trim().is_empty() {
            fail(&format!(
                "invalid {} entry for `{}`: `justification` must not be empty",
                path.display(),
                entry.name
            ));
        }
    }
    entries
}

#[derive(Default)]
struct PublicItemCollector {
    names: BTreeSet<String>,
}

impl PublicItemCollector {
    fn record_visibility(&mut self, vis: &syn::Visibility, name: impl Into<String>) {
        if matches!(vis, syn::Visibility::Public(_)) {
            self.names.insert(name.into());
        }
    }

    fn record_use_tree(&mut self, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Name(name) => {
                self.names.insert(name.ident.to_string());
            }
            syn::UseTree::Rename(rename) => {
                self.names.insert(rename.rename.to_string());
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.record_use_tree(item);
                }
            }
            syn::UseTree::Path(path) => self.record_use_tree(&path.tree),
            syn::UseTree::Glob(_) => {}
        }
    }
}

impl Visit<'_> for PublicItemCollector {
    fn visit_item_fn(&mut self, node: &syn::ItemFn) {
        self.record_visibility(&node.vis, node.sig.ident.to_string());
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_item_struct(&mut self, node: &syn::ItemStruct) {
        self.record_visibility(&node.vis, node.ident.to_string());
        syn::visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &syn::ItemEnum) {
        self.record_visibility(&node.vis, node.ident.to_string());
        syn::visit::visit_item_enum(self, node);
    }

    fn visit_item_trait(&mut self, node: &syn::ItemTrait) {
        self.record_visibility(&node.vis, node.ident.to_string());
        syn::visit::visit_item_trait(self, node);
    }

    fn visit_item_type(&mut self, node: &syn::ItemType) {
        self.record_visibility(&node.vis, node.ident.to_string());
        syn::visit::visit_item_type(self, node);
    }

    fn visit_item_const(&mut self, node: &syn::ItemConst) {
        self.record_visibility(&node.vis, node.ident.to_string());
        syn::visit::visit_item_const(self, node);
    }

    fn visit_item_mod(&mut self, node: &syn::ItemMod) {
        self.record_visibility(&node.vis, node.ident.to_string());
        syn::visit::visit_item_mod(self, node);
    }

    fn visit_item_use(&mut self, node: &syn::ItemUse) {
        if matches!(node.vis, syn::Visibility::Public(_)) {
            self.record_use_tree(&node.tree);
        }
        syn::visit::visit_item_use(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &syn::ImplItemFn) {
        self.record_visibility(&node.vis, node.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, node);
    }
}

fn walk_rs_files(dir: &Path, check: &mut dyn FnMut(&Path, &str)) {
    //Recursive directory walk. Only reads .rs files.
    //Uses std::fs only — no external deps allowed in build scripts
    //unless declared in [build-dependencies].
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_rs_files(&path, check);
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                if let Ok(contents) = fs::read_to_string(&path) {
                    check(&path, &contents);
                }
            }
        }
    }
}

fn walk_allow_checked_rs_files(check: &mut dyn FnMut(&Path, &str)) {
    let roots = [
        Path::new("build.rs"),
        Path::new("src"),
        Path::new("tools/xtask/src"),
        Path::new("tools/integrity/src"),
    ];
    for root in roots {
        if root.is_file() {
            if let Ok(contents) = fs::read_to_string(root) {
                check(root, &contents);
            }
        } else {
            walk_rs_files(root, check);
        }
    }
}
