# Contributing

listello is a personal project with one maintainer, and it was written almost entirely by AI
(see the README). That is worth knowing before you spend time here.

## What that means for you

- **There is one reviewer, and he is not a programmer.** The release gate is "does this work
  on a real Touch Bar". Expect slow answers, little ceremony, and less code review than you are
  used to. If AI-written code is a dealbreaker for you, that is a reasonable position, and this
  is the right moment to close the tab.
- **The design decisions in [`docs/DESIGN.md`](docs/DESIGN.md) are settled.** They are written
  down with their reasoning and their rejected alternatives specifically so they do not get
  relitigated. A PR that quietly reverses one will be closed; an issue arguing that one is
  wrong is welcome.

## Before opening a pull request

Open an issue first, so a PR does not collide with work already in progress.

## Do not take listello's bugs to tiny-dfr

listello is a fork of [tiny-dfr](https://github.com/AsahiLinux/tiny-dfr), and retains its DRM,
backlight, input and font code. **If you hit a bug here, report it here.** Do not open issues
or pull requests against tiny-dfr for it, even if the code involved is clearly theirs. They did
not ask for this fork, they have asked that AI agents stay out of their repository, and they
should not have to triage its bug reports. This applies to
listello's own fixes too: they stay in this repository. See `docs/DESIGN.md` D2a.

If your bug also reproduces on stock tiny-dfr, on a machine with listello not installed, then
it is genuinely their bug and reporting it to them is fine — but say nothing about listello
while you do it.

## Gates

Anything that lands has to pass, on Linux:

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build
cargo test
```

Rendering changes are additionally gated by golden images
(`LISTELLO_GOLDEN_UPDATE=1 cargo test` records new ones; look at every PNG it writes before you commit
it).

## Hardware

Touch Bar hardware is scarce and the maintainer has exactly one machine
(MacBookPro16,1, T2). If you have a Touch Bar Mac running Linux — especially Apple Silicon
under Asahi, or a 13"/15" model with no physical Esc key — testing on it is the single most
useful thing you can contribute.

## Licence

By contributing you agree that your contribution is licensed under the MIT licence, matching
the rest of the project.
