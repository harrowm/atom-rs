//! Saving the screen as a PNG picture.

/// Encode `rgba` (4 bytes per pixel, `width` x `height`) as a PNG file.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}

/// A file name for a screenshot taken at `seconds_since_1970` (UTC), such as
/// `atom-20261126-153045.png`: it sorts in the order they were taken.
pub fn file_name(seconds_since_1970: u64) -> String {
    let days = (seconds_since_1970 / 86_400) as i64;
    let in_day = seconds_since_1970 % 86_400;
    let (year, month, day) = civil_date(days);
    format!(
        "atom-{year:04}{month:02}{day:02}-{:02}{:02}{:02}.png",
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60
    )
}

/// The calendar date `days` after 1 January 1970 (Howard Hinnant's algorithm).
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_png_decodes_back_to_the_same_pixels() {
        let (width, height) = (4, 3);
        let rgba: Vec<u8> = (0..width * height * 4).map(|i| (i * 7) as u8).collect();
        let bytes = encode_png(width, height, &rgba).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");

        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!((info.width, info.height), (width, height));
        assert_eq!(&decoded[..rgba.len()], &rgba[..]);
    }

    #[test]
    fn the_wrong_amount_of_pixel_data_is_an_error_not_a_bad_file() {
        assert!(encode_png(4, 3, &[0; 10]).is_err());
    }

    #[test]
    fn file_names_carry_the_date_and_time() {
        assert_eq!(file_name(0), "atom-19700101-000000.png");
        assert_eq!(file_name(1_700_000_000), "atom-20231114-221320.png");
        // A leap day, and the end of a year.
        assert_eq!(file_name(1_709_164_800), "atom-20240229-000000.png");
        assert_eq!(file_name(1_767_225_599), "atom-20251231-235959.png");
    }

    #[test]
    fn names_sort_in_the_order_the_pictures_were_taken() {
        assert!(file_name(1_700_000_000) < file_name(1_700_000_001));
        assert!(file_name(1_709_164_800) < file_name(1_767_225_599));
    }
}
