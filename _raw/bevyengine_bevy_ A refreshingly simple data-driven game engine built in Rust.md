---
title: "bevyengine/bevy: A refreshingly simple data-driven game engine built in Rust"
source: "https://github.com/bevyengine/bevy"
author:
published:
created: 2026-06-14
description: "A refreshingly simple data-driven game engine built in Rust - bevyengine/bevy"
tags:
  - "clippings"
---
## What is Bevy?

Bevy is a refreshingly simple data-driven game engine built in Rust. It is free and open-source forever!

## WARNING

Bevy is still in the early stages of development. Important features are missing. Documentation is sparse. A new version of Bevy containing breaking changes to the API is released [approximately once every 3 months](https://bevy.org/news/bevy-0-6/#the-train-release-schedule). We provide [migration guides](https://bevy.org/learn/migration-guides/), but we can't guarantee migrations will always be easy. Use only if you are willing to work in this environment.

**MSRV:** Bevy relies heavily on improvements in the Rust language and compiler. As a result, the Minimum Supported Rust Version (MSRV) is generally close to "the latest stable release" of Rust.

## Design Goals

- **Capable**: Offer a complete 2D and 3D feature set
- **Simple**: Easy for newbies to pick up, but infinitely flexible for power users
- **Data Focused**: Data-oriented architecture using the Entity Component System paradigm
- **Modular**: Use only what you need. Replace what you don't like
- **Fast**: App logic should run quickly, and when possible, in parallel
- **Productive**: Changes should compile quickly... waiting isn't fun

## About

- **[Features](https://bevy.org/):** A quick overview of Bevy's features.
- **[News](https://bevy.org/news/)**: A development blog that covers our progress, plans and shiny new features.

## Docs

- **[Quick Start Guide](https://bevy.org/learn/quick-start/introduction):** Bevy's official Quick Start Guide. The best place to start learning Bevy.
- **[Bevy Rust API Docs](https://docs.rs/bevy):** Bevy's Rust API docs, which are automatically generated from the doc comments in this repo.
- **[Official Examples](https://github.com/bevyengine/bevy/tree/latest/examples):** Bevy's dedicated, runnable examples, which are great for digging into specific concepts.
- **[Community-Made Learning Resources](https://bevy.org/assets/#learning)**: More tutorials, documentation, and examples made by the Bevy community.

## Community

Before contributing or participating in discussions with the community, you should familiarize yourself with our [**Code of Conduct**](https://github.com/bevyengine/bevy/blob/main/CODE_OF_CONDUCT.md).

- **[Discord](https://discord.gg/bevy):** Bevy's official discord server.
- **[Reddit](https://reddit.com/r/bevy):** Bevy's official subreddit.
- **[GitHub Discussions](https://github.com/bevyengine/bevy/discussions):** The best place for questions about Bevy, answered right here!
- **[Bevy Assets](https://bevy.org/assets/):** A collection of awesome Bevy projects, tools, plugins and learning materials.

### Contributing

If you'd like to help build Bevy, check out the **[Contributor's Guide](https://bevy.org/learn/contribute/introduction)**. For simple problems, feel free to [open an issue](https://github.com/bevyengine/bevy/issues) or [PR](https://github.com/bevyengine/bevy/pulls) and tackle it yourself!

For more complex architecture decisions and experimental mad science, please open an [RFC](https://github.com/bevyengine/rfcs) (Request For Comments) so we can brainstorm together effectively!

## Getting Started

We recommend checking out the [Quick Start Guide](https://bevy.org/learn/quick-start/introduction) for a brief introduction.

Follow the [Setup guide](https://bevy.org/learn/quick-start/getting-started/setup) to ensure your development environment is set up correctly. Once set up, you can quickly try out the [examples](https://github.com/bevyengine/bevy/tree/latest/examples) by cloning this repo and running the following commands:

```
# Switch to the correct version (latest release, default is main development branch)
git checkout latest
# Runs the "breakout" example
cargo run --example breakout
```

To draw a window with standard functionality enabled, use:

```
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .run();
}
```

### Fast Compiles

Bevy can be built just fine using default configuration on stable Rust. However for really fast iterative compiles, you should enable the "fast compiles" setup by [following the instructions here](https://bevy.org/learn/quick-start/getting-started/setup).

## Bevy Cargo Features

This [list](https://github.com/bevyengine/bevy/blob/main/docs/cargo_features.md) outlines the different cargo features supported by Bevy. These allow you to customize the Bevy feature set for your use-case.

## Thanks

Bevy is the result of the hard work of many people. A huge thanks to all Bevy contributors, the many open source projects that have come before us, the [Rust gamedev ecosystem](https://arewegameyet.rs/), and the many libraries we build on.

A huge thanks to Bevy's [generous sponsors](https://bevy.org/). Bevy will always be free and open source, but it isn't free to make. Please consider [sponsoring our work](https://bevy.org/donate/) if you like what we're building.

This project is tested with BrowserStack.

## License

Bevy is free, open source and permissively licensed! Except where noted (below and/or in individual files), all code in this repository is dual-licensed under either:

- MIT License ([LICENSE-MIT](https://github.com/bevyengine/bevy/blob/main/LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](https://github.com/bevyengine/bevy/blob/main/LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))

at your option. This means you can select the license you prefer! This dual-licensing approach is the de-facto standard in the Rust ecosystem and there are [very good reasons](https://github.com/bevyengine/bevy/issues/2373) to include both.

Some of the engine's code carries additional copyright notices and license terms due to their external origins. These are generally BSD-like, but exact details vary by crate: If the README of a crate contains a 'License' header (or similar), the additional copyright notices and license terms applicable to that crate will be listed. The above licensing requirement still applies to contributions to those crates, and sections of those crates will carry those license terms. The [license](https://doc.rust-lang.org/cargo/reference/manifest.html#the-license-and-license-file-fields) field of each crate will also reflect this.

The [assets](https://github.com/bevyengine/bevy/blob/main/assets) included in this repository (for our [examples](https://github.com/bevyengine/bevy/blob/main/examples/README.md)) typically fall under different open licenses. These will not be included in your game (unless copied in by you), and they are not distributed in the published bevy crates. See [CREDITS.md](https://github.com/bevyengine/bevy/blob/main/CREDITS.md) for the details of the licenses of those files.

### Your contributions

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.