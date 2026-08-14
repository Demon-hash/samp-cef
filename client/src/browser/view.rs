use cef_sys::cef_rect_t;
use client_api::gta::matrix::CRect;
use client_api::gta::rw;
use client_api::gta::rw::rwcore::{RwRaster, RwTexture};
use client_api::gta::rw::rwplcore::{self, RwRGBA};
use client_api::gta::sprite::Sprite;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

pub struct RwLockGuard<'a> {
    bytes: &'a mut [u8],
    pub pitch: usize,
    raster: NonNull<RwRaster>,
}

impl RwLockGuard<'_> {
    #[inline(always)]
    pub fn bytes_as_mut_ptr(&mut self) -> *mut u8 {
        self.bytes.as_mut_ptr()
    }
}

impl Deref for RwLockGuard<'_> {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        self.bytes
    }
}

impl DerefMut for RwLockGuard<'_> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [u8] {
        self.bytes
    }
}

impl Drop for RwLockGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            self.raster.as_mut().unlock();
        }
    }
}

pub struct RwContainer {
    texture: Option<NonNull<RwTexture>>,
    raster: Option<NonNull<RwRaster>>,
}

impl RwContainer {
    pub fn new(width: usize, height: usize) -> RwContainer {
        let raster = RwRaster::new(width as i32, height as i32);
        let texture = RwTexture::new(raster);

        RwContainer {
            texture: NonNull::new(texture),
            raster: NonNull::new(raster),
        }
    }

    #[inline]
    pub fn bytes(&mut self) -> Option<RwLockGuard<'_>> {
        unsafe {
            let raster = self.raster.as_mut()?;
            let raster_ref = raster.as_mut();
            let bytes = raster_ref.lock(0);
            if bytes.is_null() {
                tracing::warn!("RenderWare raster lock returned a null pointer");
                return None;
            }

            let pitch = match usize::try_from(raster_ref.stride) {
                Ok(pitch) => pitch,
                Err(_) => {
                    tracing::warn!(
                        stride = raster_ref.stride,
                        "invalid RenderWare raster stride"
                    );
                    raster_ref.unlock();
                    return None;
                }
            };
            let height = match usize::try_from(raster_ref.height) {
                Ok(height) => height,
                Err(_) => {
                    tracing::warn!(
                        height = raster_ref.height,
                        "invalid RenderWare raster height"
                    );
                    raster_ref.unlock();
                    return None;
                }
            };
            let Some(size) = pitch.checked_mul(height) else {
                tracing::warn!(pitch, height, "RenderWare raster size overflow");
                raster_ref.unlock();
                return None;
            };

            Some(RwLockGuard {
                bytes: std::slice::from_raw_parts_mut(bytes, size),
                pitch,
                raster: *raster,
            })
        }
    }
}

impl Drop for RwContainer {
    fn drop(&mut self) {
        unsafe {
            // RwTexture::new() takes ownership of the raster passed to it -
            // RwTextureDestroy() (the real engine function) already frees
            // its raster internally as part of tearing down the texture.
            // `raster` here is only a convenience handle for bytes()/lock()
            // while the container is alive, not a second, independently-
            // owned object - destroying it separately after the texture is
            // already gone is a double free on the same RwRaster, which is
            // exactly what a captured crash dump showed: RtlFreeHeap
            // succeeding for the texture's destroy, immediately followed by
            // a heap-corruption write crashing inside the second, redundant
            // free.
            if let Some(mut texture) = self.texture.take() {
                texture.as_mut().destroy();
                self.raster.take();
            } else if let Some(mut raster) = self.raster.take() {
                // No texture was ever created around this raster (NonNull::new
                // returned None for `texture` in `new()`) - it's still ours
                // to free directly.
                raster.as_mut().destroy();
            }
        }
    }
}

pub struct SpriteContainer {
    sprite: Sprite,
    rw: RwContainer,
}

impl SpriteContainer {
    pub fn new(width: usize, height: usize) -> SpriteContainer {
        let rw = RwContainer::new(width, height);
        let mut sprite = Sprite::new();

        if let Some(texture) = rw.texture {
            sprite.set_texture(texture.as_ptr());
        }

        SpriteContainer { sprite, rw }
    }

    #[inline]
    pub fn draw(&mut self) {
        let client = crate::utils::client_rect();
        let rect = CRect {
            top: 0.0,
            left: 0.0,
            right: client[0] as f32,
            bottom: client[1] as f32,
        };

        let color = RwRGBA {
            red: 0xFF,
            green: 0xFF,
            blue: 0xFF,
            alpha: 0xFF,
        };

        let prev = rw::render_state(rwplcore::RENDERSTATETEXTUREFILTER);

        rw::set_render_state(rwplcore::RENDERSTATETEXTUREFILTER, rwplcore::FILTERNEAREST);
        self.sprite.draw(rect, color);

        rw::set_render_state(rwplcore::RENDERSTATETEXTUREFILTER, prev);
    }

    #[inline]
    pub fn bytes(&mut self) -> Option<RwLockGuard<'_>> {
        self.rw.bytes()
    }
}

enum ViewContainer {
    Material(RwContainer),
    Display(SpriteContainer),
}

impl ViewContainer {
    fn draw(&mut self) {
        if let ViewContainer::Display(sprite) = self {
            sprite.draw();
        }
    }

    fn bytes(&mut self) -> Option<RwLockGuard<'_>> {
        match self {
            ViewContainer::Display(sprite) => sprite.bytes(),
            ViewContainer::Material(sprite) => sprite.bytes(),
        }
    }

    fn texture(&self) -> Option<NonNull<RwTexture>> {
        match self {
            ViewContainer::Display(sprite) => sprite.rw.texture,
            ViewContainer::Material(rw) => rw.texture,
        }
    }
}

pub struct View {
    container: Option<ViewContainer>,
    width: usize,
    height: usize,
    active: bool,
}

impl View {
    pub fn new() -> View {
        View {
            container: None,
            width: 0,
            height: 0,
            active: true,
        }
    }

    pub fn make_display(&mut self, width: usize, height: usize) {
        let width = std::cmp::max(1, width);
        let height = std::cmp::max(1, height);

        self.destroy_previous();

        self.container = Some(ViewContainer::Display(SpriteContainer::new(width, height)));

        self.set_size(width, height);
    }

    #[inline(never)]
    pub fn make_inactive(&mut self) {
        self.destroy_previous();
        self.set_size(1, 1);
        self.active = false;
    }

    pub fn make_active(&mut self) {
        self.active = true;
    }

    #[inline]
    pub fn draw(&mut self) {
        if let Some(rw) = self.container.as_mut() {
            rw.draw()
        }
    }

    #[inline(always)]
    pub fn update_texture(&mut self, bytes: &[u8], rects: &[cef_rect_t]) -> bool {
        if let Some(mut dest) = self.container.as_mut().and_then(|rw| rw.bytes()) {
            let destination_pitch = dest.pitch;
            let source_pitch = self.width.saturating_mul(4);
            let source_len = bytes.len();
            let dest_len = dest.len();
            let dest = &mut *dest;

            let dest = dest.as_mut_ptr();
            let pixels_origin = bytes.as_ptr();

            for cef_rect in rects {
                // CEF's dirty rects describe the browser's *current* size,
                // reported asynchronously via OnPaint. If a resize raced
                // with this update, the texture we're holding (`self.width`/
                // `self.height`, fixed at the last resize()) can be smaller
                // than what CEF now reports - writing rect-sized spans
                // straight into `dest`/`pixels_origin` without checking that
                // is an out-of-bounds write into whatever the RenderWare
                // raster's allocation happens to sit next to on the heap.
                // Silently drop anything that doesn't fit both buffers
                // instead of copying blind; a dropped partial repaint is far
                // cheaper than a corrupted heap that crashes somewhere
                // unrelated moments later.
                if cef_rect.x < 0 || cef_rect.y < 0 || cef_rect.width <= 0 || cef_rect.height <= 0 {
                    tracing::warn!(?cef_rect, "update_texture: negative/empty dirty rect, skipping");
                    continue;
                }

                let (x, y, w, h) = (
                    cef_rect.x as usize,
                    cef_rect.y as usize,
                    cef_rect.width as usize,
                    cef_rect.height as usize,
                );

                let Some(row_bytes) = w.checked_mul(4) else {
                    tracing::warn!(?cef_rect, "update_texture: row width overflow, skipping");
                    continue;
                };

                let fits_dest = y.checked_add(h).is_some_and(|bottom| bottom <= self.height)
                    && x.checked_add(w).is_some_and(|right| right <= self.width);

                if !fits_dest {
                    tracing::warn!(
                        ?cef_rect, width = self.width, height = self.height,
                        "update_texture: dirty rect exceeds current view size, skipping"
                    );
                    continue;
                }

                let mut out_of_bounds = false;

                for row in y..(y + h) {
                    let destination_index = destination_pitch * row + x * 4;
                    let source_index = source_pitch * row + x * 4;

                    let dest_row_ok = destination_index.checked_add(row_bytes)
                        .is_some_and(|end| end <= dest_len);
                    let source_row_ok = source_index.checked_add(row_bytes)
                        .is_some_and(|end| end <= source_len);

                    if !dest_row_ok || !source_row_ok {
                        out_of_bounds = true;
                        break;
                    }

                    unsafe {
                        let ptr = dest.add(destination_index);
                        let pixels = pixels_origin.add(source_index);
                        std::ptr::copy(pixels, ptr, row_bytes);
                    }
                }

                if out_of_bounds {
                    tracing::warn!(
                        ?cef_rect, destination_pitch, source_pitch, dest_len, source_len,
                        "update_texture: computed row exceeds buffer bounds, stopped early"
                    );
                }
            }

            true
        } else {
            false
        }
    }

    pub fn update_popup(&mut self, bytes: &[u8], popup_rect: &cef_rect_t) {
        // Same missing-bounds-check hazard as update_texture (see the
        // comment there) - CEF popups (autofill/save-password dropdowns
        // included, not just <select> elements) go through this exact
        // path via on_popup_show, so a plain password <input> is enough
        // to reach it even with no explicit popup UI on the page.
        if popup_rect.x < 0 || popup_rect.y < 0 || popup_rect.width <= 0 || popup_rect.height <= 0 {
            tracing::warn!(?popup_rect, "update_popup: negative/empty popup rect, skipping");
            return;
        }

        let width = self.width;
        let height = self.height;
        let source_len = bytes.len();

        let (x, y, w, h) = (
            popup_rect.x as usize,
            popup_rect.y as usize,
            popup_rect.width as usize,
            popup_rect.height as usize,
        );

        let Some(row_bytes) = w.checked_mul(4) else {
            tracing::warn!(?popup_rect, "update_popup: row width overflow, skipping");
            return;
        };

        let fits_view = y.checked_add(h).is_some_and(|bottom| bottom <= height)
            && x.checked_add(w).is_some_and(|right| right <= width);

        if !fits_view {
            tracing::warn!(
                ?popup_rect, width, height,
                "update_popup: popup rect exceeds current view size, skipping"
            );
            return;
        }

        let set_pixels = |dest: &mut [u8], pitch: usize| {
            let dest_len = dest.len();
            let dest = dest.as_mut_ptr();

            for row in 0..h {
                let source_index = row * row_bytes;
                let dest_index = (row + y) * pitch + x * 4;

                let source_ok = source_index.checked_add(row_bytes).is_some_and(|end| end <= source_len);
                let dest_ok = dest_index.checked_add(row_bytes).is_some_and(|end| end <= dest_len);

                if !source_ok || !dest_ok {
                    tracing::warn!(
                        ?popup_rect, pitch, dest_len, source_len,
                        "update_popup: computed row exceeds buffer bounds, stopped early"
                    );
                    break;
                }

                unsafe {
                    let surface_data = dest.add(dest_index);
                    let new_data = bytes.as_ptr().add(source_index);
                    std::ptr::copy(new_data, surface_data, row_bytes);
                }
            }
        };

        self.set_texture_bytes(set_pixels);
    }

    pub fn clear(&mut self) {
        let clear = |dest: &mut [u8], _: usize| {
            let size = dest.len();
            let dest = dest.as_mut_ptr();

            unsafe {
                std::ptr::write_bytes(dest, 0x00, size);
            }
        };

        self.set_texture_bytes(clear);
    }

    pub fn on_lost_device(&mut self) {
        self.destroy_previous();
    }

    pub fn resize(&mut self, is_object: bool, width: usize, height: usize) {
        if !self.active {
            return;
        }

        let should_replace = self.active && self.container.is_none();

        if self.width == width && self.height == height && !should_replace {
            return;
        }

        let width = std::cmp::max(1, width);
        let height = std::cmp::max(1, height);

        self.destroy_previous();
        self.set_size(width, height);

        if is_object {
            self.container = Some(ViewContainer::Material(RwContainer::new(width, height)));
        } else {
            self.container = Some(ViewContainer::Display(SpriteContainer::new(width, height)));
        }
    }

    pub fn rect(&self) -> cef_rect_t {
        let width = if self.width == 0 {
            1
        } else {
            self.width as i32
        };

        let height = if self.height == 0 {
            1
        } else {
            self.height as i32
        };

        cef_rect_t {
            width,
            height,
            x: 0,
            y: 0,
        }
    }

    pub fn rwtexture(&mut self) -> Option<NonNull<RwTexture>> {
        self.container.as_mut().and_then(|rw| rw.texture())
    }

    pub fn is_empty(&self) -> bool {
        // self.render_mode == RenderMode::Empty
        false
    }

    fn destroy_previous(&mut self) {
        self.container.take();
    }

    fn set_size(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
    }

    #[inline(always)]
    fn set_texture_bytes<F>(&mut self, mut func: F)
    where
        F: FnMut(&mut [u8], usize),
    {
        if let Some(mut bytes) = self.container.as_mut().and_then(|rw| rw.bytes()) {
            let pitch = bytes.pitch;
            func(&mut bytes, pitch);
        }
    }
}

impl Default for View {
    fn default() -> Self {
        Self::new()
    }
}
