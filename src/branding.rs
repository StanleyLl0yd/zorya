const ICON_SIZE: u32 = 64;

pub(crate) fn application_icon_rgba() -> Vec<u8> {
    let size = ICON_SIZE as usize;
    let mut pixels = Vec::with_capacity(size * size * 4);

    for y in 0..size {
        for x in 0..size {
            let px = (x as f32 + 0.5) / size as f32;
            let py = (y as f32 + 0.5) / size as f32;
            let mut color = background(px, py);

            draw_frame(&mut color, px, py);
            draw_orbit(&mut color, px, py);
            draw_rays(&mut color, px, py);
            draw_sun(&mut color, px, py);
            draw_horizon(&mut color, px, py);
            draw_orbit_marker(&mut color, px, py);

            pixels.extend([
                channel(color[0]),
                channel(color[1]),
                channel(color[2]),
                255,
            ]);
        }
    }

    pixels
}

pub(crate) const fn application_icon_size() -> u32 {
    ICON_SIZE
}

fn background(x: f32, y: f32) -> [f32; 3] {
    let base = [7.0, 16.0, 59.0];
    let violet = radial(x, y, 0.50, 0.47, 0.62);
    let lower = radial(x, y, 0.50, 0.87, 0.72);
    [
        base[0] + 21.0 * violet + 14.0 * lower,
        base[1] + 12.0 * violet + 2.0 * lower,
        base[2] + 39.0 * violet + 12.0 * lower,
    ]
}

fn draw_frame(color: &mut [f32; 3], x: f32, y: f32) {
    let inset = 0.055;
    let radius = 0.19;
    let distance = rounded_rect_border_distance(x, y, inset, radius);
    let glow = glow(distance, 0.0045, 0.035);
    let t = ((x + y) * 0.5).clamp(0.0, 1.0);
    let frame = mix3([82.0, 103.0, 255.0], [255.0, 122.0, 75.0], t);
    add(color, frame, glow * 0.95);
}

fn draw_orbit(color: &mut [f32; 3], x: f32, y: f32) {
    if y > 0.72 {
        return;
    }
    let dx = (x - 0.5) / 0.39;
    let dy = (y - 0.53) / 0.39;
    let distance = ((dx * dx + dy * dy).sqrt() - 1.0).abs() * 0.39;
    add(color, [116.0, 76.0, 255.0], glow(distance, 0.0028, 0.015) * 0.9);
}

fn draw_rays(color: &mut [f32; 3], x: f32, y: f32) {
    const RAYS: [([f32; 2], [f32; 2], f32); 11] = [
        ([0.50, 0.53], [0.50, 0.20], 0.0060),
        ([0.46, 0.54], [0.39, 0.30], 0.0034),
        ([0.42, 0.56], [0.30, 0.34], 0.0048),
        ([0.39, 0.59], [0.23, 0.44], 0.0034),
        ([0.37, 0.63], [0.18, 0.52], 0.0042),
        ([0.36, 0.67], [0.19, 0.67], 0.0030),
        ([0.54, 0.54], [0.61, 0.30], 0.0034),
        ([0.58, 0.56], [0.70, 0.34], 0.0048),
        ([0.61, 0.59], [0.77, 0.44], 0.0034),
        ([0.63, 0.63], [0.82, 0.52], 0.0042),
        ([0.64, 0.67], [0.81, 0.67], 0.0030),
    ];

    for (start, end, width) in RAYS {
        let distance = segment_distance([x, y], start, end);
        let strength = glow(distance, width, width * 4.2);
        if strength > 0.0 {
            let warm = mix3([255.0, 103.0, 60.0], [255.0, 244.0, 157.0], 0.72);
            add(color, warm, strength);
        }
    }
}

fn draw_sun(color: &mut [f32; 3], x: f32, y: f32) {
    let dx = x - 0.5;
    let dy = y - 0.62;
    let distance = (dx * dx + dy * dy).sqrt();
    let radius = 0.16;

    let aura = (1.0 - ((distance - radius) / 0.11).max(0.0)).clamp(0.0, 1.0);
    add(color, [255.0, 106.0, 45.0], aura * aura * 0.42);

    if distance <= radius {
        let t = (distance / radius).clamp(0.0, 1.0);
        let center = [255.0, 250.0, 204.0];
        let edge = [255.0, 139.0, 42.0];
        *color = mix3(center, edge, t * 0.88);
    }

    let edge_distance = (distance - radius).abs();
    add(
        color,
        [255.0, 242.0, 155.0],
        glow(edge_distance, 0.004, 0.012),
    );
}

fn draw_horizon(color: &mut [f32; 3], x: f32, y: f32) {
    let normalized = x * 2.0 - 1.0;
    let horizon = 0.635 + 0.155 * normalized * normalized;
    let distance = (y - horizon).abs();

    if y > horizon {
        *color = mix3(*color, [7.0, 16.0, 59.0], 0.94);
    }

    add(
        color,
        [255.0, 116.0, 67.0],
        glow(distance, 0.0034, 0.018),
    );
    add(
        color,
        [255.0, 244.0, 166.0],
        glow(distance, 0.0017, 0.006),
    );
}

fn draw_orbit_marker(color: &mut [f32; 3], x: f32, y: f32) {
    let distance = (((x - 0.5).powi(2) + (y - 0.154).powi(2)).sqrt() - 0.013).abs();
    let inside = ((x - 0.5).powi(2) + (y - 0.154).powi(2)).sqrt() <= 0.013;

    if inside {
        *color = [221.0, 205.0, 255.0];
    }
    add(
        color,
        [140.0, 104.0, 255.0],
        glow(distance, 0.002, 0.025) * 0.9,
    );
}

fn rounded_rect_border_distance(x: f32, y: f32, inset: f32, radius: f32) -> f32 {
    let left = inset;
    let right = 1.0 - inset;
    let top = inset;
    let bottom = 1.0 - inset;

    let cx = x.clamp(left + radius, right - radius);
    let cy = y.clamp(top + radius, bottom - radius);
    let corner_distance = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
    (corner_distance - radius).abs()
}

fn segment_distance(point: [f32; 2], start: [f32; 2], end: [f32; 2]) -> f32 {
    let vx = end[0] - start[0];
    let vy = end[1] - start[1];
    let wx = point[0] - start[0];
    let wy = point[1] - start[1];
    let vv = vx * vx + vy * vy;
    let t = if vv == 0.0 {
        0.0
    } else {
        ((wx * vx + wy * vy) / vv).clamp(0.0, 1.0)
    };
    let dx = point[0] - (start[0] + t * vx);
    let dy = point[1] - (start[1] + t * vy);
    (dx * dx + dy * dy).sqrt()
}

fn radial(x: f32, y: f32, cx: f32, cy: f32, radius: f32) -> f32 {
    let dx = x - cx;
    let dy = y - cy;
    (1.0 - (dx * dx + dy * dy).sqrt() / radius).clamp(0.0, 1.0)
}

fn glow(distance: f32, core: f32, spread: f32) -> f32 {
    if distance <= core {
        1.0
    } else {
        (1.0 - (distance - core) / spread).clamp(0.0, 1.0).powi(2)
    }
}

fn add(color: &mut [f32; 3], source: [f32; 3], strength: f32) {
    if strength <= 0.0 {
        return;
    }

    for channel in 0..3 {
        color[channel] = color[channel] + (source[channel] - color[channel]) * strength.clamp(0.0, 1.0);
    }
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_icon_has_expected_rgba_shape() {
        let pixels = application_icon_rgba();

        assert_eq!(pixels.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn application_icon_contains_dark_and_sunrise_regions() {
        let pixels = application_icon_rgba();
        let mut darkest = 255_u8;
        let mut brightest_warm = false;

        for pixel in pixels.chunks_exact(4) {
            darkest = darkest.min(pixel[0]).min(pixel[1]).min(pixel[2]);
            if pixel[0] > 230 && pixel[1] > 130 && pixel[2] < 150 {
                brightest_warm = true;
            }
        }

        assert!(darkest < 20);
        assert!(brightest_warm);
    }
}
