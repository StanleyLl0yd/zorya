use std::io::Cursor;

const ICON_PNG: &[u8] = include_bytes!("../assets/branding/zorya-icon.png");

pub(crate) struct ApplicationIcon {
    pub(crate) rgba: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) fn application_icon() -> Result<ApplicationIcon, String> {
    let mut decoder = png::Decoder::new(Cursor::new(ICON_PNG));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);

    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("failed to read embedded Zorya PNG: {error}"))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("failed to decode embedded Zorya PNG: {error}"))?;
    let decoded = &buffer[..info.buffer_size()];

    let rgba = match info.color_type {
        png::ColorType::Rgba => decoded.to_vec(),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for pixel in decoded.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
            rgba
        }
        png::ColorType::GrayscaleAlpha => {
            let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for pixel in decoded.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
            rgba
        }
        png::ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
            for value in decoded {
                rgba.extend_from_slice(&[*value, *value, *value, 255]);
            }
            rgba
        }
        png::ColorType::Indexed => {
            return Err("embedded Zorya PNG remained indexed after PNG expansion".into());
        }
    };

    let expected_len = info.width as usize * info.height as usize * 4;
    if rgba.len() != expected_len {
        return Err(format!(
            "embedded Zorya PNG decoded to {} RGBA bytes; expected {expected_len}",
            rgba.len()
        ));
    }

    Ok(ApplicationIcon {
        rgba,
        width: info.width,
        height: info.height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_png_is_embedded_at_original_dimensions() {
        assert_eq!(&ICON_PNG[..8], b"\x89PNG\r\n\x1a\n");

        let icon = application_icon().expect("decode canonical Zorya PNG");
        assert_eq!((icon.width, icon.height), (1254, 1254));
        assert_eq!(
            icon.rgba.len(),
            icon.width as usize * icon.height as usize * 4
        );
    }
}
