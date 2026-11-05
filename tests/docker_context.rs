//! The Docker image compiles the crate from a reduced build context: the
//! `.dockerignore` excludes `deploy/`, so every file `src/setup/assets.rs`
//! embeds with `include_str!` must be re-included there and copied by the
//! Dockerfile, or the image build (the Dagger `docker_build` gate) fails.

use std::path::Path;

use mmhmem::setup::assets::ALL;

fn read(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_embedded_asset_reaches_the_docker_build() {
    let ignore = read(".dockerignore");
    let ignore: Vec<&str> = ignore.lines().map(str::trim).collect();
    let dockerfile = read("Dockerfile");
    for asset in ALL {
        let source = asset.source;
        assert!(
            ignore.contains(&format!("!{source}").as_str()),
            ".dockerignore must re-include {source}"
        );
        assert!(
            dockerfile
                .lines()
                .any(|line| line.starts_with(&format!("COPY {source} "))),
            "the Dockerfile must COPY {source}"
        );
    }
    let copies = dockerfile
        .lines()
        .filter(|line| line.starts_with("COPY deploy/"))
        .count();
    assert_eq!(
        copies,
        ALL.len(),
        "the Dockerfile copies only the embedded files"
    );
}
