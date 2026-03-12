build:
  cargo build --release

clippy:
  cargo clippy

test:
  cargo nextest run

publish:
  cargo publish

clean:
  cargo clean
  rm -rf pkg/ src/agentcat-git/ agentcat-git/ *.pkg.tar.zst
