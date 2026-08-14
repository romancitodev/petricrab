set shell := ["nu", "-c"]

[private]
default:
  just --list

run:
  cargo run -p petricrab-app

fmt:
  cargo fmt --all

test filter:
  cargo test {{filter}}
