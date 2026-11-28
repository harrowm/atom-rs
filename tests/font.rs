//! Checks on the character font: where the patterns sit in the cell, that every
//! character is distinct, and the shapes of the letters and digits that appear
//! in screenshots and video of real Acorn Atoms (an Atom's "ACORN ATOM",
//! "ERROR 94", `>10 PRINT"THIS` and `>LIST`, and `LOAD"EARLY WARNING"`).

use atom_rs::machine::font::{FONT, GLYPH_HEIGHT};

fn glyph(code: usize) -> &'static [u8] {
    &FONT[code * GLYPH_HEIGHT..(code + 1) * GLYPH_HEIGHT]
}

/// Draw a glyph as text, to compare against what a person can see.
fn picture(code: usize) -> Vec<String> {
    glyph(code)
        .iter()
        .map(|&row| {
            (0..8)
                .map(|x| if row & (0x80 >> x) != 0 { '#' } else { '.' })
                .collect()
        })
        .collect()
}

/// What a phone video of a real Atom showed of `G` and `W` (see
/// the video), as features that a blurry picture can still tell apart.
#[test]
fn g_and_w_match_the_video_of_a_real_atom() {
    let g = picture(code_of('G'));
    // A flat top: the top bar is four dots, starting one dot in from the
    // left stem, and the row under it is the stem alone, open at the right.
    assert_eq!(g[3], "...####.");
    assert_eq!(g[4], "..#.....");
    // The specimen's round-topped G would be "...###.." and "..#...#.".
    let w = picture(code_of('W'));
    // The centre stroke of the W is already there on the fourth row.
    assert_eq!(w[3 + 3], "..#.#.#.");
    // ...and not on the third.
    assert_eq!(w[3 + 2], "..#...#.");
    // F is E without its foot: the same top and middle bars.
    let f = picture(code_of('F'));
    assert_eq!(&f[3..9], &picture(code_of('E'))[3..9]);
    assert_eq!(f[9], "..#.....");
    // The E that the same video showed: the middle bar one dot shorter than the top.
    let e = picture(code_of('E'));
    assert_eq!(e[3], "..#####.");
    assert_eq!(e[6], "..####..");
}

#[test]
fn every_pattern_sits_where_the_data_sheet_puts_it() {
    // Two columns in from the left of the 8-wide cell, five wide: bits 5-1.
    // Three rows down, seven high: scanlines 3-9.
    for code in 0..64 {
        for (scanline, &row) in glyph(code).iter().enumerate() {
            assert_eq!(
                row & !0x3E,
                0,
                "code {code} scanline {scanline} strays outside columns 2-6"
            );
            if !(3..=9).contains(&scanline) {
                assert_eq!(row, 0, "code {code} scanline {scanline} should be blank");
            }
        }
    }
}

#[test]
fn only_the_space_is_blank_and_no_two_characters_look_alike() {
    for code in 0..64 {
        let blank = glyph(code).iter().all(|&row| row == 0);
        assert_eq!(blank, code == 32, "code {code}");
    }
    for a in 0..64 {
        for b in (a + 1)..64 {
            assert_ne!(glyph(a), glyph(b), "codes {a} and {b} are the same");
        }
    }
}

/// The code of a character, in the data sheet's order (Figure 20).
fn code_of(c: char) -> usize {
    match c {
        '@'..='_' => c as usize - 0x40,
        ' '..='?' => c as usize,
        _ => panic!("{c} is not in the character set"),
    }
}

/// The glyphs that appear in a screenshot of a real Acorn Atom (its start-up
/// banner `ACORN ATOM`, `ERROR 94`, and a typed line `>10 PRINT"THIS`,
/// `>LIST`), compared with it by eye when the font was made. Each is the
/// seven scanlines (3 to 9) of the pattern, in the 8-pixel cell.
#[test]
fn the_glyphs_match_what_a_real_atom_shows() {
    let seen: [(char, [&str; 7]); 20] = [
        (
            'A',
            [
                "....#...", "...#.#..", "..#...#.", "..#...#.", "..#####.", "..#...#.", "..#...#.",
            ],
        ),
        (
            'C',
            [
                "...###..", "..#...#.", "..#.....", "..#.....", "..#.....", "..#...#.", "...###..",
            ],
        ),
        (
            'O',
            [
                "..#####.", "..#...#.", "..#...#.", "..#...#.", "..#...#.", "..#...#.", "..#####.",
            ],
        ),
        (
            'R',
            [
                "..####..", "..#...#.", "..#...#.", "..####..", "..#.#...", "..#..#..", "..#...#.",
            ],
        ),
        (
            'N',
            [
                "..#...#.", "..##..#.", "..#.#.#.", "..#..##.", "..#...#.", "..#...#.", "..#...#.",
            ],
        ),
        (
            'T',
            [
                "..#####.", "....#...", "....#...", "....#...", "....#...", "....#...", "....#...",
            ],
        ),
        (
            'M',
            [
                "..#...#.", "..##.##.", "..#.#.#.", "..#.#.#.", "..#...#.", "..#...#.", "..#...#.",
            ],
        ),
        (
            'E', // (the middle bar is four dots, as measured on a real Atom)
            [
                "..#####.", "..#.....", "..#.....", "..####..", "..#.....", "..#.....", "..#####.",
            ],
        ),
        (
            'P',
            [
                "..####..", "..#...#.", "..#...#.", "..####..", "..#.....", "..#.....", "..#.....",
            ],
        ),
        (
            'H',
            [
                "..#...#.", "..#...#.", "..#...#.", "..#####.", "..#...#.", "..#...#.", "..#...#.",
            ],
        ),
        (
            'I',
            [
                "...###..", "....#...", "....#...", "....#...", "....#...", "....#...", "...###..",
            ],
        ),
        (
            'S',
            [
                "...###..", "..#...#.", "...#....", "....#...", ".....#..", "..#...#.", "...###..",
            ],
        ),
        (
            'L',
            [
                "..#.....", "..#.....", "..#.....", "..#.....", "..#.....", "..#.....", "..#####.",
            ],
        ),
        (
            '9',
            [
                "...###..", "..#...#.", "..#...#.", "...####.", "......#.", "......#.", "...###..",
            ],
        ),
        (
            '4',
            [
                ".....#..", "....##..", "...#.#..", "..#####.", ".....#..", ".....#..", ".....#..",
            ],
        ),
        (
            '1',
            [
                "....#...", "...##...", "....#...", "....#...", "....#...", "....#...", "...###..",
            ],
        ),
        (
            '0',
            [
                "...##...", "..#..#..", "..#..#..", "..#..#..", "..#..#..", "..#..#..", "...##...",
            ],
        ),
        (
            '"',
            [
                "...#.#..", "...#.#..", "...#.#..", "........", "........", "........", "........",
            ],
        ),
        (
            '>',
            [
                "...#....", "....#...", ".....#..", "......#.", ".....#..", "....#...", "...#....",
            ],
        ),
        (' ', ["........"; 7]),
    ];
    for (character, expected) in seen {
        assert_eq!(&picture(code_of(character))[3..10], expected, "{character}");
    }
}

#[test]
fn letters_that_are_symmetrical_are_symmetrical() {
    // Reflect a row within the five columns (bits 5..1) and compare.
    let mirror = |row: u8| (0..5).fold(0, |acc, i| acc | (((row >> (1 + i)) & 1) << (5 - i)));
    // H, I, O, T, U, V, W, X, Y, A, M, !, +, 8, 0 are all left-right symmetric.
    for letter in "HIOTUVWXYAM".chars() {
        let code = letter as usize - 0x40;
        for &row in glyph(code) {
            assert_eq!(mirror(row), row, "{letter} is not symmetrical");
        }
    }
    // F, L, P and R are not.
    for letter in "FLPR".chars() {
        let code = letter as usize - 0x40;
        assert!(
            glyph(code).iter().any(|&row| mirror(row) != row),
            "{letter}"
        );
    }
}

#[test]
fn the_character_order_is_the_data_sheets() {
    // Figure 20: row 0_ is @ A ... O, 1_ is P ... Z [ \ ] up left, 2_ is
    // space ! " # $ % & ' ( ) * + , - . / and 3_ is 0-9 : ; < = > ?.
    // The glyphs above were checked by shape, so check the places of a few
    // others by what must be true of them.
    let lit_rows = |c: char| {
        picture(code_of(c))
            .iter()
            .filter(|r| r.contains('#'))
            .count()
    };
    assert_eq!(
        lit_rows('!'),
        6,
        "a stem of five dots, a gap, then the point"
    );
    assert_eq!(lit_rows(' '), 0);
    assert!(
        picture(code_of('-'))
            .iter()
            .filter(|r| r.contains('#'))
            .count()
            == 1,
        "a dash is one row"
    );
    assert!(
        picture(code_of('.'))
            .iter()
            .filter(|r| r.contains('#'))
            .count()
            <= 2,
        "a point is small"
    );
    // Letters and digits: a letter's code is its place in the alphabet.
    assert_eq!(code_of('A'), 1);
    assert_eq!(code_of('Z'), 26);
    assert_eq!(code_of('0'), 0x30);
    assert_eq!(code_of('?'), 0x3F);
}

// ---- The image reader --------------------------------------------------------------
