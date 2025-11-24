use std::collections::HashSet;
use std::env;
use std::path::PathBuf;

use bindgen::callbacks::{DeriveInfo, MacroParsingBehavior, ParseCallbacks};

#[derive(Debug)]
struct BindgenRules {
    ingore_macros: HashSet<String>,
}

impl ParseCallbacks for BindgenRules {
    fn will_parse_macro(&self, name: &str) -> MacroParsingBehavior {
        if self.ingore_macros.contains(name) {
            MacroParsingBehavior::Ignore
        } else {
            MacroParsingBehavior::Default
        }
    }

    fn add_derives(&self, info: &DeriveInfo<'_>) -> Vec<String> {
        match info.name {
            "raft_result" => vec!["PartialEq".to_owned(), "Eq".to_owned()],
            _ => vec![],
        }
    }
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("rustc-link-lib=static=dqlite");

    let lib = pkg_config::Config::new()
        .statik(true)
        .atleast_version("1.18.4")
        // Override idiotic decision from pkg_config maintainer to silently ignore the user will and 
        // link dynamically when the library is found in /usr. 
        .cargo_metadata(false)
        .probe("dqlite")
        .expect("Failed to find libdqlite");

    // This does not handle all use cases, but handles the dqlite one.
    for include_path in lib.include_paths {
        println!("cargo:include={}", include_path.display());
    }

    for path in lib.link_paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }

    for lib in lib.libs {
        if lib == "dqlite" {
            println!("cargo:rustc-link-lib=static={}", lib);
            continue;
        }
        println!("cargo:rustc-link-lib={}", lib);
    }

    let bindings = bindgen::Builder::default()
        .header("dqlite-internal.h")
        .new_type_alias("raft_result")
        .constified_enum_module("raft_result_code")
        .constified_enum_module("raft_role")
        .constified_enum_module("raft_entry_type")
        .constified_enum_module("raft_command_type")
        .parse_callbacks(Box::new(BindgenRules {
            ingore_macros: HashSet::new(),
        }))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .derive_default(true)
        .derive_debug(false)
        .derive_copy(true)
        .no_copy("raft_configuration")
        .no_copy("raft_snapshot")
        .no_copy("uvSegmentBuffer")
        .generate()
        .expect("Unable to generate bindings");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
