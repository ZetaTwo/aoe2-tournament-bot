# List available recipes
default:
    @just --list

# List outdated dependencies: crates, npm packages, then images, actions and toolchain pins (--all: list current ones too). Needs cargo-outdated.
outdated *args:
    -cargo outdated --root-deps-only
    python3 scripts/outdated.py {{ args }}
