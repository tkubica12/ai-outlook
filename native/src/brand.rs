pub const CORNERS: [[[f32; 2]; 3]; 4] = [
    [[22.0, 10.0], [10.0, 10.0], [10.0, 22.0]],
    [[42.0, 10.0], [54.0, 10.0], [54.0, 22.0]],
    [[10.0, 42.0], [10.0, 54.0], [22.0, 54.0]],
    [[54.0, 42.0], [54.0, 54.0], [42.0, 54.0]],
];
pub const LETTER: [[f32; 4]; 2] = [[19.0, 21.0, 45.0, 27.0], [29.0, 24.0, 35.0, 47.0]];
pub const DEFAULT_COLOR: [u8; 3] = [0, 105, 161];

pub fn rgba(size: usize) -> Vec<u8> {
    rgba_with_color(size, DEFAULT_COLOR)
}

pub fn rgba_with_color(size: usize, color: [u8; 3]) -> Vec<u8> {
    let mut pixels = vec![0; size * size * 4];
    let pixel_width = 64.0 / size as f32;
    for y in 0..size {
        for x in 0..size {
            let px = (x as f32 + 0.5) * pixel_width;
            let py = (y as f32 + 0.5) * pixel_width;
            let mut distance = f32::INFINITY;
            for corner in CORNERS {
                for segment in corner.windows(2) {
                    let dx = px
                        - px.clamp(
                            segment[0][0].min(segment[1][0]),
                            segment[0][0].max(segment[1][0]),
                        );
                    let dy = py
                        - py.clamp(
                            segment[0][1].min(segment[1][1]),
                            segment[0][1].max(segment[1][1]),
                        );
                    distance = distance.min((dx * dx + dy * dy).sqrt() - 3.0);
                }
            }
            for [left, top, right, bottom] in LETTER {
                distance =
                    distance.min((left - px).max(px - right).max((top - py).max(py - bottom)));
            }
            let alpha = ((0.5 - distance / pixel_width).clamp(0.0, 1.0) * 255.0).round() as u8;
            if alpha != 0 {
                pixels[(y * size + x) * 4..][..4]
                    .copy_from_slice(&[color[0], color[1], color[2], alpha]);
            }
        }
    }
    pixels
}

pub fn ico() -> Vec<u8> {
    let sizes = [16_u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let mut frames = Vec::new();
    for size in sizes {
        let rgba = rgba(size as usize);
        let mask_stride = size.div_ceil(32) * 4;
        let mut frame = Vec::new();
        frame.extend(40_u32.to_le_bytes());
        frame.extend(size.to_le_bytes());
        frame.extend((size * 2).to_le_bytes());
        frame.extend(1_u16.to_le_bytes());
        frame.extend(32_u16.to_le_bytes());
        frame.extend([0_u8; 24]);
        for y in (0..size as usize).rev() {
            for x in 0..size as usize {
                let pixel = &rgba[(y * size as usize + x) * 4..][..4];
                frame.extend([pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }
        let mut mask = vec![0; (mask_stride * size) as usize];
        for y in 0..size as usize {
            for x in 0..size as usize {
                if rgba[(y * size as usize + x) * 4 + 3] == 0 {
                    mask[(size as usize - 1 - y) * mask_stride as usize + x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
        frame.extend(mask);
        frames.push(frame);
    }
    let mut icon = vec![0, 0, 1, 0];
    icon.extend((sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + sizes.len() as u32 * 16;
    for (size, frame) in sizes.into_iter().zip(&frames) {
        // ICO uses zero in a dimension byte to represent 256 pixels.
        icon.extend([size as u8, size as u8, 0, 0]);
        icon.extend(1_u16.to_le_bytes());
        icon.extend(32_u16.to_le_bytes());
        icon.extend((frame.len() as u32).to_le_bytes());
        icon.extend(offset.to_le_bytes());
        offset += frame.len() as u32;
    }
    for frame in frames {
        icon.extend(frame);
    }
    icon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_rasters_keep_the_same_silhouette_and_transparent_background_at_all_icon_sizes() {
        for size in [16, 20, 24, 32, 40, 48, 64, 128, 256] {
            let pixels = rgba_with_color(size, [17, 83, 149]);
            assert_eq!(pixels.len(), size * size * 4);
            assert_eq!(&pixels[..4], &[0, 0, 0, 0]);
            assert_eq!(&pixels[pixels.len() - 4..], &[0, 0, 0, 0]);
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] == 255)
            );
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| (1..255).contains(&pixel[3]))
            );
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| pixel[3] == 0 || pixel[..3] == [17, 83, 149])
            );
        }
        let pixels = rgba(64);
        let alpha = |x: usize, y: usize| pixels[(y * 64 + x) * 4 + 3];
        for (x, y) in [(10, 10), (54, 10), (10, 54), (54, 54), (32, 24), (32, 40)] {
            assert_eq!(alpha(x, y), 255);
        }
        for (x, y) in [(32, 10), (32, 54), (10, 32), (54, 32), (22, 40), (42, 40)] {
            assert_eq!(alpha(x, y), 0);
        }
    }

    #[test]
    fn changing_accent_changes_only_color_not_geometry_or_opacity() {
        let first = rgba_with_color(32, [0, 105, 161]);
        let second = rgba_with_color(32, [242, 197, 68]);
        assert!(
            first
                .as_chunks::<4>()
                .0
                .iter()
                .zip(second.as_chunks::<4>().0.iter())
                .all(|(a, b)| a[3] == b[3])
        );
        assert_ne!(first, second);
    }

    #[test]
    fn executable_icon_contains_decodable_focus_frames_for_each_windows_size() {
        let icon = ico();
        assert_eq!(&icon[..6], &[0, 0, 1, 0, 9, 0]);
        for index in 0..9 {
            let entry = &icon[6 + index * 16..][..16];
            let size = if entry[0] == 0 {
                256
            } else {
                usize::from(entry[0])
            };
            let length = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
            let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
            let mut single = vec![0, 0, 1, 0, 1, 0];
            single.extend(&entry[..12]);
            single.extend(22_u32.to_le_bytes());
            single.extend(&icon[offset..offset + length]);
            let decoded = image::load_from_memory_with_format(&single, image::ImageFormat::Ico)
                .unwrap()
                .into_rgba8();
            assert_eq!(decoded.dimensions(), (size as u32, size as u32));
            assert_eq!(decoded.into_raw(), rgba(size));
        }
    }
}
