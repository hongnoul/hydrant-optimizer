# Hydrant Optimizer

An exact MIT course-schedule optimizer in Rust. Choose your subjects, then minimize **days with classes first, gaps between classes second**.

Optimality covers only known, supported meeting times. See [optimizer details](OPTIMIZER.MD).

## Install and run

Requires Git and [current stable Rust](https://rustup.rs) (1.88+).

```sh
git clone https://github.com/hongnoul/hydrant-optimizer.git
cd hydrant-optimizer
cargo install --path . --locked
hydrant-optimizer
```

Opens a terminal UI. First launch downloads the catalog and requires internet. Press `?` for controls or `q` to quit.

If the command is not found, add `~/.cargo/bin` to your `PATH`.
