//! Captured stroke storage: raster RGBA or one stable mask slot.
use crate::{
    document::{Document, Layer, LayerId},
    pixels,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Target {
    #[default]
    Raster,
    Mask(LayerId),
}
impl Target {
    pub(crate) fn is_raster(&self) -> bool {
        matches!(self, Self::Raster)
    }
    pub(crate) fn valid(self, layer: &Layer, size: [usize; 2]) -> bool {
        match self {
            Self::Raster => !layer.folder && layer.pixels.len() == size[0] * size[1] * 4,
            Self::Mask(id) => layer
                .mask
                .as_ref()
                .is_some_and(|m| m.id == id && m.coverage.dimensions() == size),
        }
    }
    pub(crate) fn blocker(self, doc: &Document, index: usize, erase: bool) -> Option<&'static str> {
        let Some(layer) = doc.layers.get(index) else {
            return Some("Captured painting target no longer exists");
        };
        if !self.valid(layer, [doc.width, doc.height]) {
            Some("Captured painting target no longer exists")
        } else if layer.locked {
            Some("Unlock the layer before painting")
        } else if !doc.shown(index) {
            Some("Show the layer and its folder before painting")
        } else if self.is_raster() && layer.text.is_some() {
            Some(crate::text_layer::BLOCKER)
        } else if self.is_raster() && erase && layer.alpha_locked {
            Some("Unlock transparency before erasing")
        } else {
            None
        }
    }
    pub(crate) fn read(self, layer: &Layer, width: usize, x: usize, y: usize) -> [u8; 4] {
        match self {
            Self::Raster => layer.pixels.rgba(y * width + x),
            Self::Mask(_) => {
                let value = layer.mask.as_ref().unwrap().coverage.at(x, y);
                [value, value, value, 255]
            }
        }
    }
    pub(crate) fn writer(self, layer: &mut Layer, width: usize) -> Writer<'_> {
        match self {
            Self::Raster => Writer::Raster(layer.pixels.writer(), width),
            Self::Mask(_) => Writer::Mask(layer.mask.as_mut().unwrap().coverage.writer()),
        }
    }
    pub(crate) fn params(
        self,
        color: [u8; 3],
        erase: bool,
        alpha_locked: bool,
    ) -> ([u8; 3], bool, bool) {
        match self {
            Self::Raster => (color, erase, alpha_locked),
            Self::Mask(_) => {
                let gray = if erase {
                    255
                } else {
                    ((u32::from(color[0]) * 77
                        + u32::from(color[1]) * 150
                        + u32::from(color[2]) * 29
                        + 128)
                        >> 8) as u8
                };
                ([gray; 3], false, true)
            }
        }
    }
    pub(crate) fn side(self) -> usize {
        if self.is_raster() { pixels::SIDE } else { 64 }
    }
    pub(crate) fn reclaim(self, layer: &mut Layer, width: usize, rect: [usize; 4]) {
        match self {
            Self::Raster => layer.pixels.writer().reclaim_region(width, rect),
            Self::Mask(_) => layer
                .mask
                .as_mut()
                .unwrap()
                .coverage
                .writer()
                .reclaim_region(rect),
        }
    }
}
pub(crate) enum Writer<'a> {
    Raster(pixels::Writer<'a>, usize),
    Mask(crate::layer_mask::Writer<'a>),
}
impl Writer<'_> {
    /// Masks already stage individual gray pixels; short flat rows have no
    /// tile-ownership overhead to amortize with a coverage buffer.
    pub(crate) fn prefers_scalar_dabs(&self, width: usize) -> bool {
        matches!(self, Self::Mask(_))
            || (width < 32 && matches!(self, Self::Raster(pixels::Writer::Flat(_), _)))
    }

    pub(crate) fn edit_span(
        &mut self,
        x: usize,
        y: usize,
        count: usize,
        mut edit: impl FnMut(usize, &mut [u8]),
    ) {
        match self {
            Self::Raster(writer, width) => writer.edit_span(*width, x, y, count, edit),
            Self::Mask(_) => {
                for offset in 0..count {
                    self.edit(x + offset, y, |pixel| edit(offset, pixel));
                }
            }
        }
    }

    pub(crate) fn edit<R>(&mut self, x: usize, y: usize, edit: impl FnOnce(&mut [u8]) -> R) -> R {
        match self {
            Self::Raster(writer, width) => edit(writer.rgba_mut_at(*width, x, y)),
            Self::Mask(writer) => {
                let value = writer.at(x, y);
                let mut rgba = [value, value, value, 255];
                let result = edit(&mut rgba);
                writer.set(x, y, rgba[0]);
                result
            }
        }
    }
    pub(crate) fn set(&mut self, x: usize, y: usize, rgba: &[u8]) {
        self.edit(x, y, |pixel| pixel.copy_from_slice(rgba));
    }
}
