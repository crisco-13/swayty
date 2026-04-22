# swayty

swayty (pronounced "sweaty") is a SwayWM daemon that forces your desktop environment (and you) to suffer alongside your hardware. 

Modern computers hide their struggle behind quiet fans and abstract task managers, swayty fixes this by providing anxiety inducing visual feedback when your CPU is under load. Written in Rust for a completely memory safe swayting experience.

## Should I use swayty?

You should consider using swayty if you:

- Are tired of conventional system monitors
- Want to be visually punished for opening another Electron app
- Like to suffer
- Like to watch others suffer
- Would rather look at pretty colors and funny animations over boring numbers
- Are *really* bored

## Requirements

*   SwayWM
*   A CPU capable of feeling pain

## Installation

You will need Rust and Cargo installed.

Clone the repository and build the project:

```sh
git clone https://github.com/crisco-13/swayty.git
cd swayty
cargo build --release
```

Place the compiled binary somewhere in your `$PATH`. For most setups, the following will work:

```sh
cp target/release/swayty ~/.local/bin/
```

## Usage

You can start it manually from your terminal to start swayting:

```sh
swayty
```

To make the pain permanent, add the following line to your Sway configuration file (usually `~/.config/sway/config`):

```swayconfig
exec swayty
```

## Configuration

There is no configuration file because the suffering is hardcoded. 

## License

MIT. See the LICENSE file for details.
