//! Conservative screen-space coverage for isolated two-dimensional blends.
use crate::mesh::as_mesh;
use ruffle_render::commands::{Command, CommandList, RenderBlendMode};
use ruffle_render::matrix::Matrix;
use ruffle_render::transform::Transform;
use swf::{BlendMode, Twips};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BlendRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl BlendRegion {
    pub fn full(width: u32, height: u32) -> Self { Self { x: 0, y: 0, width, height } }
    pub fn pixels(self) -> u64 { u64::from(self.width) * u64::from(self.height) }

    pub fn pooled(self, parent_width: u32, parent_height: u32) -> Self {
        #[cfg(test)]
        if FORCE_EXACT_BLEND_SIZE.get() { return self; }
        if self.width == 0 || self.height == 0 { return self; }
        // Keep the original integer origin and raster scale. Only clear extra
        // pixels on the right/bottom so nearby animation sizes share a pool key.
        let round = |size, available| (u64::from(size).div_ceil(16) * 16)
            .min(u64::from(available)) as u32;
        let padded = Self { width: round(self.width, parent_width - self.x),
            height: round(self.height, parent_height - self.y), ..self };
        // Tiny targets should not pay an arbitrarily large pixel-area penalty.
        if padded.pixels() > self.pixels() * 2 { self } else { padded }
    }
}

pub(super) fn local_blend(mode: &RenderBlendMode) -> bool {
    !matches!(mode, RenderBlendMode::Shader(_) | RenderBlendMode::Builtin(BlendMode::Alpha | BlendMode::Erase))
}

#[derive(Default)]
struct Bounds(Option<[f64; 4]>);

impl Bounds {
    fn rect(&mut self, matrix: Matrix, rect: [f32; 4]) -> Option<()> {
        let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for x in [rect[0], rect[2]] { for y in [rect[1], rect[3]] {
            let px = f64::from(matrix.a) * f64::from(x) + f64::from(matrix.c) * f64::from(y) + matrix.tx.to_pixels();
            let py = f64::from(matrix.b) * f64::from(x) + f64::from(matrix.d) * f64::from(y) + matrix.ty.to_pixels();
            if !px.is_finite() || !py.is_finite() { return None; }
            bounds[0] = bounds[0].min(px); bounds[1] = bounds[1].min(py);
            bounds[2] = bounds[2].max(px); bounds[3] = bounds[3].max(py);
        }}
        if let Some(old) = &mut self.0 {
            old[0] = old[0].min(bounds[0]); old[1] = old[1].min(bounds[1]);
            old[2] = old[2].max(bounds[2]); old[3] = old[3].max(bounds[3]);
        } else { self.0 = Some(bounds); }
        Some(())
    }

    fn commands(&mut self, commands: &CommandList) -> Option<()> {
        fn matrix(transform: &Transform) -> Option<Matrix> {
            (transform.tz == 0.0 && transform.perspective_projection.is_none()).then_some(transform.matrix)
        }
        for command in &commands.commands {
            match command {
                Command::RenderBitmap { transform, pixel_snapping, region, .. } => {
                    let mut matrix = matrix(transform)?;
                    pixel_snapping.apply(&mut matrix);
                    self.rect(matrix, [0.0, 0.0, region.width() as f32, region.height() as f32])?;
                }
                Command::RenderShape { shape, transform } => self.rect(matrix(transform)?, as_mesh(shape).bounds?)?,
                Command::DrawRect { matrix, .. } | Command::DrawLine { matrix, .. } | Command::DrawLineRect { matrix, .. } => {
                    self.rect(*matrix, [0.0, 0.0, 1.0, 1.0])?;
                }
                Command::Blend(inner, mode) if local_blend(mode) => self.commands(inner)?,
                Command::PushMask | Command::ActivateMask | Command::DeactivateMask | Command::PopMask => (),
                // Layer-dependent alpha/erase, arbitrary shader reads, alpha masks
                // and perspective draws retain their original coordinate domain.
                _ => return None,
            }
        }
        Some(())
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static FORCE_FULL_BLEND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(crate) static FORCE_EXACT_BLEND_SIZE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// None means unsupported, while a zero-sized region is safely outside the target.
pub(super) fn region(commands: &CommandList, width: u32, height: u32) -> Option<BlendRegion> {
    #[cfg(test)]
    if FORCE_FULL_BLEND.get() { return None; }
    let mut bounds = Bounds::default();
    bounds.commands(commands)?;
    let Some([left, top, right, bottom]) = bounds.0 else { return Some(BlendRegion::full(0, 0)); };
    // Include antialiasing and the one-pixel device-space line caps. Filtered
    // bitmaps already include the filter's expanded output in their region.
    let x = (left.floor() - 2.0).clamp(0.0, f64::from(width)) as u32;
    let y = (top.floor() - 2.0).clamp(0.0, f64::from(height)) as u32;
    let right = (right.ceil() + 2.0).clamp(0.0, f64::from(width)) as u32;
    let bottom = (bottom.ceil() + 2.0).clamp(0.0, f64::from(height)) as u32;
    Some(BlendRegion { x, y, width: right.saturating_sub(x), height: bottom.saturating_sub(y) })
}

pub(super) fn translate(commands: &mut CommandList, region: BlendRegion) {
    let x = Twips::from_pixels(f64::from(region.x));
    let y = Twips::from_pixels(f64::from(region.y));
    for command in &mut commands.commands {
        let matrix = match command {
            Command::RenderBitmap { transform, .. } | Command::RenderShape { transform, .. } => &mut transform.matrix,
            Command::DrawRect { matrix, .. } | Command::DrawLine { matrix, .. } | Command::DrawLineRect { matrix, .. } => matrix,
            Command::Blend(inner, _) => { translate(inner, region); continue; }
            _ => continue,
        };
        matrix.tx -= x; matrix.ty -= y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pooled_regions_preserve_origin_coverage_and_parent_edges() {
        for parent_width in [1, 16, 63, 1200] {
            for x in 0..parent_width {
                for width in [0, 1, 7, 17, 35, 100] {
                    let original = BlendRegion { x, y: 5, width: width.min(parent_width-x), height: 19 };
                    let padded = original.pooled(parent_width, 31);
                    assert_eq!((padded.x,padded.y),(original.x,original.y));
                    assert!(padded.width >= original.width && padded.height >= original.height);
                    assert!(padded.x+padded.width <= parent_width && padded.y+padded.height <= 31);
                    assert!(padded.pixels() <= original.pixels()*2);
                }
            }
        }
        let a=BlendRegion {x:2,y:2,width:35,height:23};
        let b=BlendRegion {width:42,height:27,..a};
        assert_eq!(a.pooled(1200,700),b.pooled(1200,700));
    }
    fn rectangle(matrix: Matrix) -> CommandList {
        let mut list = CommandList::new();
        list.commands.push(Command::DrawRect { color: swf::Color::WHITE, matrix }); list
    }
    #[test]
    fn clips_negative_edges_and_preserves_device_padding() {
        let commands = rectangle(Matrix { a: 8.0, d: 7.0, tx: Twips::from_pixels(-3.5), ty: Twips::from_pixels(10.25), ..Matrix::IDENTITY });
        assert_eq!(region(&commands,1200,700), Some(BlendRegion {x:0,y:8,width:7,height:12}));
    }
    #[test]
    fn rotated_and_reflected_rectangles_include_all_corners() {
        let commands = rectangle(Matrix { a:-8.0,b:3.0,c:2.0,d:7.0,tx:Twips::from_pixels(20.0),ty:Twips::from_pixels(30.0) });
        assert_eq!(region(&commands,100,100),Some(BlendRegion {x:10,y:28,width:14,height:14}));
    }
    #[test]
    fn offstage_commands_need_no_target() {
        let commands = rectangle(Matrix { tx:Twips::from_pixels(2000.0), ..Matrix::IDENTITY });
        assert_eq!(region(&commands,1200,700).unwrap().width,0);
    }
    #[test]
    fn nested_layer_alpha_keeps_original_coordinate_domain() {
        let mut commands=CommandList::new();
        commands.commands.push(Command::Blend(rectangle(Matrix::IDENTITY),RenderBlendMode::Builtin(BlendMode::Alpha)));
        assert!(region(&commands,1200,700).is_none());
    }
    #[test]
    fn translation_keeps_relative_coordinates_inside_nested_groups() {
        let mut commands=CommandList::new();
        commands.commands.push(Command::Blend(rectangle(Matrix {tx:Twips::from_pixels(20.0),ty:Twips::from_pixels(30.0),..Matrix::IDENTITY}),RenderBlendMode::Builtin(BlendMode::Screen)));
        let bounds=region(&commands,1200,700).unwrap();translate(&mut commands,bounds);
        if let Command::Blend(inner,_) = &commands.commands[0] {
            if let Command::DrawRect {matrix,..} = &inner.commands[0] { assert_eq!(matrix.tx.to_pixels(),2.0);assert_eq!(matrix.ty.to_pixels(),2.0); }
            else {panic!()}
        } else {panic!()}
    }
    #[test]
    fn nonfinite_coordinates_use_original_path() {
        assert!(region(&rectangle(Matrix {a:f32::NAN,..Matrix::IDENTITY}),100,100).is_none());
    }
}
