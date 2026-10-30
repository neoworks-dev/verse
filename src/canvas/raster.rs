//! Draws egui's triangle meshes on the CPU into a shared-memory buffer, so the menu needs no GPU
//! context. egui already anti-aliases by feathering its meshes, so plain triangle filling with
//! texture sampling is all that's needed.

use std::collections::HashMap;

use egui::{
    Color32, TextureFilter,
    epaint::{ClippedPrimitive, ImageData, ImageDelta, Primitive, TextureId, Vertex, textures::TexturesDelta},
};

struct Texture {
    width: usize,
    height: usize,
    pixels: Vec<Color32>,
    linear: bool,
}

#[derive(Default)]
pub struct Raster {
    textures: HashMap<TextureId, Texture>,
}

impl Raster {
    pub fn set_textures(&mut self, delta: &TexturesDelta) {
        for (id, deltas) in &delta.set {
            for d in deltas {
                self.set(*id, d);
            }
        }
    }

    /// Called after painting, as egui expects.
    pub fn free_textures(&mut self, delta: &TexturesDelta) {
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    fn set(&mut self, id: TextureId, delta: &ImageDelta) {
        let ImageData::Color(image) = &delta.image;
        let [w, h] = image.size;
        match delta.pos {
            None => {
                let linear = delta.options.magnification == TextureFilter::Linear;
                self.textures.insert(id, Texture { width: w, height: h, pixels: image.pixels.clone(), linear });
            }
            Some([x0, y0]) => {
                let Some(tex) = self.textures.get_mut(&id) else { return };
                for y in 0..h {
                    let dst = (y0 + y) * tex.width + x0;
                    tex.pixels[dst..dst + w].copy_from_slice(&image.pixels[y * w..(y + 1) * w]);
                }
            }
        }
    }

    /// Paints into `target`: premultiplied BGRA (wl_shm's ARGB8888), `width` pixels per row.
    pub fn paint(&self, primitives: &[ClippedPrimitive], ppp: f32, target: &mut [u8], width: usize, height: usize) {
        target.fill(0);
        for prim in primitives {
            let Primitive::Mesh(mesh) = &prim.primitive else { continue };
            let Some(tex) = self.textures.get(&mesh.texture_id) else { continue };
            let clip = [
                (prim.clip_rect.min.x * ppp).floor().max(0.0) as i32,
                (prim.clip_rect.min.y * ppp).floor().max(0.0) as i32,
                (prim.clip_rect.max.x * ppp).ceil().min(width as f32) as i32,
                (prim.clip_rect.max.y * ppp).ceil().min(height as f32) as i32,
            ];
            if clip[0] >= clip[2] || clip[1] >= clip[3] {
                continue;
            }
            for tri in mesh.indices.chunks_exact(3) {
                let v = [&mesh.vertices[tri[0] as usize], &mesh.vertices[tri[1] as usize], &mesh.vertices[tri[2] as usize]];
                triangle(v, ppp, clip, tex, target, width);
            }
        }
    }
}

fn edge(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

/// Whether pixels exactly on edge a→b belong to this triangle. The same edge walked the other
/// way (by the neighbouring triangle) gets the opposite answer, so no pixel is blended twice.
fn owns_edge(a: [f32; 2], b: [f32; 2]) -> bool {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    dy > 0.0 || (dy == 0.0 && dx < 0.0)
}

fn triangle(v: [&Vertex; 3], ppp: f32, clip: [i32; 4], tex: &Texture, target: &mut [u8], width: usize) {
    let mut v = v;
    let pos = |v: &Vertex| [v.pos.x * ppp, v.pos.y * ppp];
    let mut p = [pos(v[0]), pos(v[1]), pos(v[2])];
    let mut area = edge(p[0], p[1], p[2]);
    if area.abs() < 1e-6 {
        return;
    }
    if area < 0.0 {
        v.swap(1, 2);
        p.swap(1, 2);
        area = -area;
    }

    let x0 = (p[0][0].min(p[1][0]).min(p[2][0]).floor() as i32).max(clip[0]);
    let y0 = (p[0][1].min(p[1][1]).min(p[2][1]).floor() as i32).max(clip[1]);
    let x1 = (p[0][0].max(p[1][0]).max(p[2][0]).ceil() as i32).min(clip[2]);
    let y1 = (p[0][1].max(p[1][1]).max(p[2][1]).ceil() as i32).min(clip[3]);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let owns = [owns_edge(p[1], p[2]), owns_edge(p[2], p[0]), owns_edge(p[0], p[1])];
    let colors = v.map(|v| v.color.to_array().map(|c| c as f32));
    let uniform_color = v[0].color == v[1].color && v[1].color == v[2].color;
    let uniform_uv = v[0].uv == v[1].uv && v[1].uv == v[2].uv;
    let flat_texel = uniform_uv.then(|| sample(tex, v[0].uv.x, v[0].uv.y));

    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let row = y as usize * width;
        for x in x0..x1 {
            let pt = [x as f32 + 0.5, py];
            let e = [edge(p[1], p[2], pt), edge(p[2], p[0], pt), edge(p[0], p[1], pt)];
            if (0..3).any(|i| e[i] < 0.0 || (e[i] == 0.0 && !owns[i])) {
                continue;
            }
            let w = [e[0] / area, e[1] / area, e[2] / area];

            let color = if uniform_color {
                colors[0]
            } else {
                std::array::from_fn(|c| w[0] * colors[0][c] + w[1] * colors[1][c] + w[2] * colors[2][c])
            };
            let texel = flat_texel.unwrap_or_else(|| {
                let u = w[0] * v[0].uv.x + w[1] * v[1].uv.x + w[2] * v[2].uv.x;
                let t = w[0] * v[0].uv.y + w[1] * v[1].uv.y + w[2] * v[2].uv.y;
                sample(tex, u, t)
            });

            // Both are premultiplied, so the product is too.
            let src: [f32; 4] = std::array::from_fn(|c| color[c] * texel[c] / 255.0);
            let i = (row + x as usize) * 4;
            let keep = 1.0 - src[3] / 255.0;
            // RGBA source onto BGRA target.
            for (dst, s) in [(i + 2, src[0]), (i + 1, src[1]), (i, src[2]), (i + 3, src[3])] {
                target[dst] = (s + target[dst] as f32 * keep).round().min(255.0) as u8;
            }
        }
    }
}

fn sample(tex: &Texture, u: f32, v: f32) -> [f32; 4] {
    let texel = |x: usize, y: usize| tex.pixels[y * tex.width + x].to_array().map(|c| c as f32);
    let fx = u * tex.width as f32 - 0.5;
    let fy = v * tex.height as f32 - 0.5;
    if !tex.linear {
        let x = (fx + 0.5).clamp(0.0, (tex.width - 1) as f32) as usize;
        let y = (fy + 0.5).clamp(0.0, (tex.height - 1) as f32) as usize;
        return texel(x, y);
    }
    let x0 = fx.floor().clamp(0.0, (tex.width - 1) as f32);
    let y0 = fy.floor().clamp(0.0, (tex.height - 1) as f32);
    let (tx, ty) = ((fx - x0).clamp(0.0, 1.0), (fy - y0).clamp(0.0, 1.0));
    let (x0, y0) = (x0 as usize, y0 as usize);
    let (x1, y1) = ((x0 + 1).min(tex.width - 1), (y0 + 1).min(tex.height - 1));
    let (a, b, c, d) = (texel(x0, y0), texel(x1, y0), texel(x0, y1), texel(x1, y1));
    std::array::from_fn(|i| {
        let top = a[i] + (b[i] - a[i]) * tx;
        let bottom = c[i] + (d[i] - c[i]) * tx;
        top + (bottom - top) * ty
    })
}
