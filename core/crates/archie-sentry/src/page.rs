//! Text laid out as pages a printer accepts.
//!
//! Home printers that speak IPP take pictures (JPEG, PWG raster) and often nothing else:
//! not plain text, frequently not PDF. So text is drawn here, one A4 page to a JPEG, in
//! an 8×8 fixed-width face enlarged three times. It is plain, and it is legible.
use font8x8::{UnicodeFonts, BASIC_FONTS, LATIN_FONTS};
use jpeg_encoder::{ColorType, Encoder};

/// A4 at 200 dots to the inch.
const WIDTH: usize = 1654;
const HEIGHT: usize = 2339;
const MARGIN: usize = 120;
const SCALE: usize = 3;
const ADVANCE: usize = 8 * SCALE;
const LEADING: usize = 12 * SCALE;
pub const COLUMNS: usize = (WIDTH - 2 * MARGIN) / ADVANCE;
pub const LINES: usize = (HEIGHT - 2 * MARGIN) / LEADING;
pub const MAX_PAGES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PageError {
    #[error("there is nothing to print")]
    Empty,
    #[error("the text needs {0} pages, and at most {MAX_PAGES} are printed at once")]
    TooLong(usize),
    #[error("the page could not be encoded")]
    Encoding,
}

/// Breaks text into lines no wider than the page, at spaces where there are any.
pub fn wrap(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.replace('\t', "    ").lines() {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let mut word: Vec<char> = word.chars().filter(|c| !c.is_control()).collect();
            let fits = line.chars().count() + usize::from(!line.is_empty()) + word.len() <= COLUMNS;
            if !fits && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // A word wider than the page is cut; nothing is dropped.
            while word.len() > COLUMNS {
                lines.push(word.drain(..COLUMNS).collect());
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.extend(word);
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

fn glyph(character: char) -> [u8; 8] {
    BASIC_FONTS
        .get(character)
        .or_else(|| LATIN_FONTS.get(character))
        .or_else(|| BASIC_FONTS.get('?'))
        .unwrap_or([0; 8])
}

fn draw(lines: &[String]) -> Vec<u8> {
    let mut page = vec![255u8; WIDTH * HEIGHT];
    for (row, line) in lines.iter().enumerate() {
        for (column, character) in line.chars().enumerate() {
            let (left, top) = (MARGIN + column * ADVANCE, MARGIN + row * LEADING);
            for (y, bits) in glyph(character).iter().enumerate() {
                for x in (0..8).filter(|x| bits >> x & 1 == 1) {
                    for dy in 0..SCALE {
                        let start = (top + y * SCALE + dy) * WIDTH + left + x * SCALE;
                        page[start..start + SCALE].fill(0);
                    }
                }
            }
        }
    }
    page
}

/// The text as JPEG pages, in order.
pub fn pages(text: &str) -> Result<Vec<Vec<u8>>, PageError> {
    let lines = wrap(text);
    if lines.is_empty() {
        return Err(PageError::Empty);
    }
    let count = lines.len().div_ceil(LINES);
    if count > MAX_PAGES {
        return Err(PageError::TooLong(count));
    }
    lines
        .chunks(LINES)
        .map(|chunk| {
            let mut jpeg = Vec::new();
            Encoder::new(&mut jpeg, 90)
                .encode(&draw(chunk), WIDTH as u16, HEIGHT as u16, ColorType::Luma)
                .map_err(|_| PageError::Encoding)?;
            Ok(jpeg)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_wrapped_at_spaces_and_a_word_too_wide_is_cut_not_dropped() {
        let long = "palabra ".repeat(20);
        let lines = wrap(long.trim());
        assert!(lines.len() > 1 && lines.iter().all(|line| line.chars().count() <= COLUMNS));
        assert_eq!(lines.join(" "), long.trim());
        let wide = "x".repeat(COLUMNS * 2 + 5);
        assert_eq!(
            wrap(&wide)
                .iter()
                .map(|l| l.chars().count())
                .collect::<Vec<_>>(),
            [COLUMNS, COLUMNS, 5]
        );
        assert_eq!(wrap("uno\n\ndos\n\n"), ["uno", "", "dos"]);
    }

    #[test]
    fn a_page_is_a_jpeg_with_ink_on_it_and_spanish_has_its_letters() {
        let one = pages("Hola, soy Archie.\nÑandú, año, canción.").unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(&one[0][..3], &[0xff, 0xd8, 0xff]);
        assert_eq!(
            crate::adapters::ipp::print::format_of(&one[0]),
            Some("image/jpeg")
        );
        assert!(draw(&["A".into()]).contains(&0));
        for letter in "áéíóúñÑ¿¡ü".chars() {
            assert_ne!(glyph(letter), glyph('?'), "{letter}");
        }
    }

    #[test]
    fn long_text_takes_more_pages_and_too_long_is_refused() {
        let two = "linea\n".repeat(LINES + 1);
        assert_eq!(pages(&two).unwrap().len(), 2);
        assert_eq!(
            pages(&"linea\n".repeat(LINES * MAX_PAGES + 1)),
            Err(PageError::TooLong(MAX_PAGES + 1))
        );
        assert_eq!(pages("  \n"), Err(PageError::Empty));
    }
}
