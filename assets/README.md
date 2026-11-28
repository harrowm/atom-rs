# Assets

## `mc6847-character-set.png`

A specimen of the Motorola MC6847's built-in character generator: the 64
characters, each a 5 x 7 dot pattern, shown at 3 times size (each dot is a 3 x 3
pixel block) in two rows of 32. The top row is codes `$20`-`$3F` (space,
punctuation, digits) and the bottom row is codes `$00`-`$1F` (`@`, `A`-`Z`,
`[`, `\`, `]`, up arrow, left arrow).

* **Source:** <https://friedorange.github.io/MatrixSans/research/MC6847.png>,
  from the "Motorola MC6847" section of
  <https://friedorange.github.io/MatrixSans/research.html> by Brad Neil
  (retrieved 28 November 2026).
* **Licence:** the page states "All images on this page are made available
  under the Creative Commons Zero licence (public domain dedication)"
  (<https://creativecommons.org/publicdomain/zero/1.0/>).
* **SHA-256:** `89dd441b98a63f61f14574bdfb30dd6ba776de02e1627783b033288e39191841`
  (the file is exactly as downloaded).

`src/machine/font.rs` is generated from this image by
`cargo run --example make_font`, and a test checks the two still agree. Where
the 5 x 7 pattern sits in the 8 x 12 character cell is taken from the MC6847
data sheet, not from the image.

## Corrections to the specimen (`src/machine/font.rs`, `CORRECTIONS`)

The specimen is not exactly the Atom's ROM. Where real Atoms disagree with it,
`font.rs` corrects it, and says why:

* **`E`, middle bar:** measured on a screenshot of a real Atom and checked on a
  phone video of another (no part of either is included here).
* **`G` and `W`:** the shapes come from `src/MC6847/CharRom.vhd` in
  <https://github.com/hoglet67/AtomGodilVideo> (Apache-2.0), because they agree
  with the same video where the specimen does not. That file states no author or
  origin for its data, so it is not known whether it was made independently.

* **`F`, middle bar:** four dots like `E`. This is from the standard MC6847
  character set on Wikipedia (`File:MC6847_Standard_Charset.png`, by Robcfg,
  CC BY-SA 4.0), which was read dot by dot as a **comparison only** and is not
  included here. After the corrections above, all 64 glyphs agree with it.
