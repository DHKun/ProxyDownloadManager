//! NSWorkspace file and type icons, rasterized to one PNG.

use std::path::Path;

use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_app_kit::{
    NSBitmapImageRep, NSCompositingOperation, NSGraphicsContext, NSImage, NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::raster::{self, ICON_PX};

pub fn file_png(path: &Path) -> Option<Vec<u8>> {
    if !path.exists() {
        return None;
    }
    let text = path.to_str()?;
    let ns = NSString::from_str(text);
    let image = unsafe { NSWorkspace::sharedWorkspace().iconForFile(&ns) };
    rasterize(&image)
}

pub fn type_png(ext: &str, _mime: &str) -> Option<Vec<u8>> {
    let token = ext.trim_start_matches('.');
    if token.is_empty() {
        return None;
    }
    let ns = NSString::from_str(token);
    let image = unsafe { NSWorkspace::sharedWorkspace().iconForFileType(&ns) };
    rasterize(&image)
}

fn rasterize(image: &NSImage) -> Option<Vec<u8>> {
    let rep = bitmap(ICON_PX)?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    let side = f64::from(ICON_PX);
    unsafe {
        let desired = NSSize {
            width: side,
            height: side,
        };
        context.saveGraphicsState();
        NSGraphicsContext::setCurrentContext(Some(&*context));
        image.setSize(desired);
        image.drawAtPoint_fromRect_operation_fraction(
            NSPoint::ZERO,
            NSRect::new(NSPoint::ZERO, desired),
            NSCompositingOperation::Copy,
            1.0,
        );
        context.flushGraphics();
        context.restoreGraphicsState();
        let len = usize::try_from(rep.bytesPerPlane()).ok()?;
        if len < (ICON_PX as usize) * (ICON_PX as usize) * 4 {
            return None;
        }
        let pixels = std::slice::from_raw_parts(rep.bitmapData(), len);
        raster::encode(
            ICON_PX,
            ICON_PX,
            &pixels[..(ICON_PX as usize) * (ICON_PX as usize) * 4],
        )
    }
}

fn bitmap(side: u32) -> Option<Retained<NSBitmapImageRep>> {
    let color = NSString::from_str("NSDeviceRGBColorSpace");
    let side = isize::try_from(side).ok()?;
    unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            side,
            side,
            8,
            4,
            true,
            false,
            &color,
            side * 4,
            32,
        )
    }
}
