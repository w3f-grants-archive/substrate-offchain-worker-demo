use std::io::Result;

fn main() -> Result<()> {
    // https://github.com/tokio-rs/prost#using-prost-in-a-no_std-crate
    // "Additionally, configure prost-build to output BTreeMaps instead of HashMaps for all Protobuf map fields in your build.rs:"
    let mut config = prost_build::Config::new();
    config.btree_map(&["."]);
    // https://docs.rs/prost-build/0.7.0/prost_build/struct.Config.html#examples-1
    // "Match all bytes fields. Expecially useful in `no_std` contexts."
    config.bytes(&["."]);

    config.compile_protos(
        // list of protos
        &[
            "deps/protos/api_circuits/api.proto",
            "deps/protos/api_circuits/circuits_routes.proto",
        ],
        // includes
        &["deps/protos"],
    )?;
    Ok(())
}

// use pb_rs::{types::FileDescriptor, ConfigBuilder};
// use std::path::{Path, PathBuf};
// use walkdir::WalkDir;

// fn main() {
//     let out_dir = std::env::var("OUT_DIR").unwrap();
//     let out_dir = Path::new(&out_dir).join(""); // TODO .join("protos")

//     // TODO compile dir above, and use correct include_paths for ConfigBuilder
//     let in_dir = PathBuf::from(::std::env::var("CARGO_MANIFEST_DIR").unwrap())
//         .join("deps/protos/api_circuits/circuits_routes.proto");
//     // Re-run this build.rs if the protos dir changes (i.e. a new file is added)
//     println!("cargo:rerun-if-changed={}", in_dir.to_str().unwrap());

//     // Find all *.proto files in the `in_dir` and add them to the list of files
//     let mut protos = Vec::new();
//     let proto_ext = Some(Path::new("proto").as_os_str());
//     for entry in WalkDir::new(&in_dir) {
//         let path = entry.unwrap().into_path();
//         if path.extension() == proto_ext {
//             // Re-run this build.rs if any of the files in the protos dir change
//             println!("cargo:rerun-if-changed={}", path.to_str().unwrap());
//             protos.push(path);
//         }
//     }

//     // Delete all old generated files before re-generating new ones
//     if out_dir.exists() {
//         std::fs::remove_dir_all(&out_dir).unwrap();
//     }
//     std::fs::DirBuilder::new().create(&out_dir).unwrap();
//     // https://github.com/tafia/quick-protobuf/pull/123
//     let config_builder = ConfigBuilder::new(&protos, None, Some(&out_dir), &[in_dir])
//         .unwrap()
//         .headers(false)
//         .error_cycle(false)
//         .nostd(true)
//         .dont_use_cow(true);
//     FileDescriptor::run(&config_builder.build()).unwrap()
// }
