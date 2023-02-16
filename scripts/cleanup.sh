set -e
cargo clippy --fix
cargo clippy --fix -- -W clippy::pedantic
cargo clippy --fix -- -W clippy::nursery
cargo fmt
cd firmware
cargo clippy --fix
cargo clippy --fix -- -W clippy::pedantic
cargo clippy --fix -- -W clippy::nursery
cargo fmt
