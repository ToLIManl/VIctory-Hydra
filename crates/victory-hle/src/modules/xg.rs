//! `xg_xs.dll`: the console's texture layout library. Titles ask it which
//! swizzle mode a texture should use, how large it is, where each mip and
//! slice lies, and to copy linear data into tiled memory and back. Served
//! from `victory_formats::texture`.
//!
//! The title imports `XGCreateTextureComputer` and
//! `XGSuggestSwizzleMode` and calls these computer methods: Release (0x08),
//! the layout (0x10), copies into (0x40) and out of (0x48) a subresource,
//! and the 64 KiB tiling (0x50). The other slots log and fail.
#![allow(non_snake_case)]
// Title code passes pointers the real library would trust.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::sync::atomic::{AtomicU32, Ordering};

use victory_formats::texture::{Dimension, Error, Layout, Swizzle, Texture, TextureDesc};

use super::xasync::{E_INVALIDARG, E_NOTIMPL};
use super::xgameruntime::{HRESULT, S_OK};
use crate::{HleModule, hle_export};

const E_FAIL: HRESULT = 0x8000_4005_u32 as i32;

/// `D3D12_RESOURCE_DESC`, whose layout field carries `0x100 | swizzle
/// mode` on the console (1 row major, 0 "pick one").
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ResourceDesc {
    pub dimension: u32,
    pub alignment: u64,
    pub width: u64,
    pub height: u32,
    pub depth_or_array: u16,
    pub mips: u16,
    pub format: u32,
    pub samples: u32,
    pub quality: u32,
    pub layout: u32,
    pub flags: u32,
}
const _: () = assert!(size_of::<ResourceDesc>() == 0x38);

const SWIZZLED: u32 = 0x100;
const ROW_MAJOR: u32 = 1;
const ALLOW_RENDER_TARGET: u32 = 1;
const ALLOW_DEPTH_STENCIL: u32 = 2;
const TEXTURE_1D: u32 = 2;
const TEXTURE_3D: u32 = 4;

/// The mode the console's library picks for a description: linear for 1D,
/// the XOR render and depth modes for targets, else 4 KiB standard
/// (`.ttpf` textures without a mode of their own use it).
///
/// shortcut: the rule of the library's older optimal-mode function (any
/// size, 2D, 3D and cubes alike); `XGSuggestSwizzleMode` itself is newer
/// than every copy we could check. Revisit if streamed textures come out
/// scrambled.
fn suggested(desc: &ResourceDesc) -> Swizzle {
    if desc.dimension == TEXTURE_1D {
        Swizzle::LINEAR
    } else if desc.flags & ALLOW_DEPTH_STENCIL != 0 {
        Swizzle::SW_64KB_Z_X
    } else if desc.flags & ALLOW_RENDER_TARGET != 0 {
        Swizzle::SW_64KB_R_X
    } else {
        Swizzle::SW_4KB_S
    }
}

/// `(desc, flags, out layout)`; the title passes flags 0 and puts the
/// result in the description's layout field.
pub extern "system" fn XGSuggestSwizzleMode(
    desc: *const ResourceDesc,
    _flags: u32,
    out: *mut u32,
) -> HRESULT {
    // SAFETY: the title passes a description and room for the layout
    // (either may be null).
    match unsafe { (desc.as_ref(), out.as_mut()) } {
        (Some(desc), Some(out)) => {
            *out = SWIZZLED | suggested(desc).0;
            S_OK
        }
        _ => E_INVALIDARG,
    }
}

#[repr(C)]
pub struct Computer {
    vtable: &'static Vtable,
    refs: AtomicU32,
    desc: ResourceDesc,
    mips: u32,
    texture: Texture,
}

type This = *mut Computer;

fn computer<'a>(this: This) -> &'a Computer {
    // SAFETY: called from computer methods only.
    unsafe { &*this }
}

pub extern "system" fn XGCreateTextureComputer(
    desc: *const ResourceDesc,
    out: *mut This,
) -> HRESULT {
    // SAFETY: the title passes a description and room for a pointer.
    let (Some(&desc), false) = (unsafe { desc.as_ref() }, out.is_null()) else {
        return E_INVALIDARG;
    };
    let Some(texture) = texture(&desc) else {
        tracing::warn!(?desc, "no texture layout");
        return E_INVALIDARG;
    };
    tracing::debug!(
        width = desc.width,
        height = desc.height,
        format = desc.format,
        layout = format_args!("{:#x}", desc.layout),
        size = texture.size,
        "texture computer"
    );
    let mips = texture.planes[0].layout.mips.len() as u32;
    let computer = Box::new(Computer {
        vtable: &VTABLE,
        refs: AtomicU32::new(1),
        desc,
        mips,
        texture,
    });
    // SAFETY: checked non-null.
    unsafe { out.write(Box::into_raw(computer)) };
    S_OK
}

fn texture(desc: &ResourceDesc) -> Option<Texture> {
    let dimension = match desc.dimension {
        TEXTURE_1D => Dimension::Tex1d,
        3 => Dimension::Tex2d,
        TEXTURE_3D => Dimension::Tex3d,
        _ => return None,
    };
    let swizzle = match desc.layout {
        0 => suggested(desc),
        ROW_MAJOR => Swizzle::LINEAR,
        // `XG_TEXTURE_LAYOUT_64KB_UNDEFINED_SWIZZLE` (2) and `64KB_STANDARD_SWIZZLE` (3).
        2 | 3 => Swizzle::SW_64KB_S,
        l if (SWIZZLED..=SWIZZLED | 0x1f).contains(&l) => Swizzle(l & 0x1f),
        _ => return None,
    };
    let texture = Texture::new(&TextureDesc {
        dimension,
        width: u32::try_from(desc.width).ok()?,
        height: desc.height,
        depth_or_array: u32::from(desc.depth_or_array),
        mips: u32::from(desc.mips),
        format: desc.format,
        samples: desc.samples.max(1),
        swizzle,
    })
    .ok()?;
    // The layout struct has room for 15 mips.
    (texture.planes[0].layout.mips.len() <= MIPS).then_some(texture)
}

const MIPS: usize = 15;

/// One mip in the layout the title reads (offsets 0x00, 0x08, 0x10 and
/// 0x1c are read by the title; the rest follow the library's
/// older public shape).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct MipOut {
    pub size: u64,
    pub offset: u64,
    pub slice_size: u64,
    pub pitch_texels: u32,
    pub pitch_bytes: u32,
    pub alignment: u32,
    /// Width, height in elements, depth or array size.
    pub padded: [u32; 3],
    pub elements: [u32; 2],
    pub texels: [u32; 2],
    pub depth_or_array: u32,
    pub samples: u32,
    pub swizzle: u32,
    pub bank_rotation: [u64; 2],
    pub slice_depth: u32,
    pub in_tail: u32,
    /// Where a tail mip starts in elements: x bits 0-10, y 11-21.
    pub tail_position: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PlaneOut {
    /// 1 data, 7 depth, 8 stencil (metadata planes are not made).
    pub usage: u32,
    pub size: u64,
    pub slice_size: u64,
    pub offset: u64,
    pub alignment: u64,
    pub element_bytes: u32,
    pub standard_mips: u32,
    pub tail_mips: u32,
    pub mips: [MipOut; MIPS],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LayoutOut {
    pub size: u64,
    pub alignment: u64,
    pub mips: u32,
    pub planes: u32,
    pub plane: [PlaneOut; 4],
    pub dimension: u32,
}
const _: () = assert!(size_of::<MipOut>() == 0x70);
const _: () = assert!(size_of::<PlaneOut>() == 0x6c8);
const _: () = assert!(size_of::<LayoutOut>() == 0x1b40);

/// The mode a tail mip is reported in: the 256-byte one of its kind (3 for
/// the render kind, which GFX10 lacks), the depth mode itself.
fn tail_swizzle(swizzle: Swizzle) -> u32 {
    match swizzle.0 {
        2 | 6 | 10 | 18 | 22 | 26 => Swizzle::SW_256B_D.0,
        23 | 27 => 3,
        24 => swizzle.0,
        _ => Swizzle::SW_256B_S.0,
    }
}

fn mip_out(desc: &ResourceDesc, l: &Layout, i: usize) -> MipOut {
    let m = &l.mips[i];
    let bpe = u64::from(l.desc.element_bytes);
    let block = l.desc.block;
    let three_d = desc.dimension == TEXTURE_3D;
    let array = u32::from(desc.depth_or_array).max(1);
    let mut out = MipOut {
        pitch_texels: m.pitch * block,
        pitch_bytes: m.pitch * l.desc.element_bytes,
        padded: [m.pitch, m.rows, if three_d { m.depth_pitch } else { array }],
        elements: [m.width, m.height],
        texels: [((desc.width >> i) as u32).max(1), (desc.height >> i).max(1)],
        depth_or_array: if three_d { m.depth } else { array },
        samples: l.desc.samples,
        slice_depth: l.block[2],
        ..MipOut::default()
    };
    let slice = u64::from(m.pitch) * u64::from(m.rows) * bpe;
    let depth = u64::from(if three_d { m.depth_pitch } else { 1 });
    match m.tail {
        None => {
            out.offset = m.offset;
            out.slice_size = slice * u64::from(l.desc.samples);
            out.size = out.slice_size * depth;
            out.alignment = l.alignment as u32;
            out.swizzle = l.desc.swizzle.0;
        }
        Some(t) => {
            out.in_tail = 1;
            out.alignment = 256;
            out.swizzle = tail_swizzle(l.desc.swizzle);
            out.tail_position = t.x | t.y << 11;
            if l.thick {
                // A 3D tail mip is placed by position, not by offset.
                out.slice_size = l.alignment / u64::from(l.block[2]);
            } else {
                out.offset = u64::from(t.offset);
                out.slice_size = slice;
            }
            out.size = out.slice_size * depth;
        }
    }
    out
}

extern "system" fn add_ref(this: This) -> u32 {
    computer(this).refs.fetch_add(1, Ordering::Relaxed) + 1
}

extern "system" fn release(this: This) -> u32 {
    let left = computer(this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        // SAFETY: made by `Box::into_raw` in `XGCreateTextureComputer`; the
        // last reference is gone.
        drop(unsafe { Box::from_raw(this) });
    }
    left
}

extern "system" fn layout(this: This, out: *mut LayoutOut) -> HRESULT {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let c = computer(this);
    // SAFETY: the title passes room for the layout; every field is plain
    // data, so zeroes are a valid start.
    let out = unsafe {
        out.write_bytes(0, 1);
        &mut *out
    };
    out.size = c.texture.size;
    out.alignment = c.texture.alignment;
    out.mips = c.mips;
    out.planes = c.texture.planes.len() as u32;
    out.dimension = c.desc.dimension;
    // The library calls a depth plane "depth" only beside stencil or
    // without mips.
    let depth =
        c.desc.flags & ALLOW_DEPTH_STENCIL != 0 && (c.texture.planes.len() > 1 || c.mips == 1);
    for (plane, p) in out.plane.iter_mut().zip(&c.texture.planes) {
        let l = &p.layout;
        plane.usage = match (p.stencil, depth) {
            (true, _) => 8,
            (false, true) => 7,
            (false, false) => 1,
        };
        plane.size = l.size;
        plane.slice_size = l.slice_size;
        plane.offset = p.offset;
        plane.alignment = l.alignment;
        plane.element_bytes = l.desc.element_bytes;
        plane.standard_mips = l.first_tail_mip;
        plane.tail_mips = c.mips - l.first_tail_mip;
        for (i, mip) in plane.mips.iter_mut().take(l.mips.len()).enumerate() {
            *mip = mip_out(&c.desc, l, i);
        }
    }
    S_OK
}

/// The plane, its mip and array slice for a subresource index, and the
/// tiled bytes and the linear bytes the copy touches.
struct CopyArgs<'a> {
    layout: &'a Layout,
    mip: u32,
    slice: u32,
    tiled: usize,
    linear: usize,
}

fn copy_args(
    c: &Computer,
    plane: u32,
    subresource: u32,
    row: u32,
    depth: u32,
) -> Option<CopyArgs<'_>> {
    let p = c.texture.planes.get(plane as usize)?;
    let l = &p.layout;
    let (mip, slice) = (subresource % c.mips, subresource / c.mips);
    let m = l.mips.get(mip as usize)?;
    let bytes = m.width as usize * l.desc.element_bytes as usize;
    let depths = l.subresource_depth(mip) as usize;
    let linear = (depths - 1) * depth as usize + (m.height as usize - 1) * row as usize + bytes;
    Some(CopyArgs {
        layout: l,
        mip,
        slice,
        tiled: (p.offset + l.size) as usize,
        linear,
    })
}

fn result(r: Result<(), Error>) -> HRESULT {
    match r {
        Ok(()) => S_OK,
        Err(Error::Unsupported) => E_NOTIMPL,
        Err(_) => E_INVALIDARG,
    }
}

/// `(tiled base, plane, subresource, linear, row pitch, depth pitch)`.
extern "system" fn copy_into(
    this: This,
    tiled: *mut u8,
    plane: u32,
    subresource: u32,
    linear: *const u8,
    row: u32,
    depth: u32,
) -> HRESULT {
    let c = computer(this);
    let Some(a) = copy_args(c, plane, subresource, row, depth) else {
        return E_INVALIDARG;
    };
    if tiled.is_null() || linear.is_null() {
        return E_INVALIDARG;
    }
    let offset = c.texture.planes[plane as usize].offset as usize;
    // SAFETY: the title's memory holds the whole texture and its rows.
    let (tiled, linear) = unsafe {
        (
            std::slice::from_raw_parts_mut(tiled.add(offset), a.tiled - offset),
            std::slice::from_raw_parts(linear, a.linear),
        )
    };
    let r = a
        .layout
        .tile(tiled, a.mip, a.slice, linear, row as usize, depth as usize);
    if r.is_err() {
        tracing::warn!(?r, desc = ?c.desc, subresource, "copy into a texture failed");
    }
    result(r)
}

/// `(linear, plane, subresource, sample, tiled base, row pitch, depth
/// pitch)`.
#[allow(clippy::too_many_arguments)]
extern "system" fn copy_from(
    this: This,
    linear: *mut u8,
    plane: u32,
    subresource: u32,
    _sample: u32,
    tiled: *const u8,
    row: u32,
    depth: u32,
) -> HRESULT {
    let c = computer(this);
    let Some(a) = copy_args(c, plane, subresource, row, depth) else {
        return E_INVALIDARG;
    };
    if tiled.is_null() || linear.is_null() {
        return E_INVALIDARG;
    }
    let offset = c.texture.planes[plane as usize].offset as usize;
    // SAFETY: the title's memory holds the whole texture and its rows.
    let (tiled, linear) = unsafe {
        (
            std::slice::from_raw_parts(tiled.add(offset), a.tiled - offset),
            std::slice::from_raw_parts_mut(linear, a.linear),
        )
    };
    let r = a
        .layout
        .detile(tiled, a.mip, a.slice, linear, row as usize, depth as usize);
    if r.is_err() {
        tracing::warn!(?r, desc = ?c.desc, subresource, "copy out of a texture failed");
    }
    result(r)
}

#[repr(C)]
pub struct PackedMips {
    pub standard: u8,
    pub packed: u8,
    pub tiles: u32,
    pub start: u32,
}

#[repr(C)]
pub struct SubresourceTiling {
    pub width: u32,
    pub height: u16,
    pub depth: u16,
    pub start: u32,
}

const TILE: u64 = 64 << 10;

/// `(total tiles, packed mips, tile shape in texels, in: room / out:
/// count, first, subresource tilings)`, in 64 KiB tiles of plane 0, for 2D
/// 64 KiB modes only (others fail, as on the console). The mip tail is the
/// packed part, tile 0 of each slice; tilings list the standard mips of
/// every slice in turn. With a tail the console's library lists two slices
/// at most and zeroes all but the first; so do we (the title reads slice 0
/// only).
extern "system" fn tiling(
    this: This,
    tiles: *mut u32,
    packed: *mut PackedMips,
    shape: *mut [u32; 3],
    count: *mut u32,
    first: u32,
    out: *mut SubresourceTiling,
) -> HRESULT {
    let c = computer(this);
    let l = &c.texture.planes[0].layout;
    if l.alignment != TILE {
        return E_FAIL;
    }
    if l.thick {
        return E_NOTIMPL;
    }
    let standard = l.first_tail_mip;
    let tail = standard < c.mips;
    let tail_tiles = match standard {
        0 => l.slice_size / TILE,
        s if s < c.mips => l.mips[s as usize - 1].offset / TILE,
        _ => 0,
    };
    let block = l.desc.block;
    // SAFETY: each output is null or room for its value; `out` has room
    // for `*count` entries.
    unsafe {
        if let Some(t) = tiles.as_mut() {
            *t = (l.size / TILE) as u32;
        }
        if let Some(p) = packed.as_mut() {
            *p = PackedMips {
                standard: standard as u8,
                packed: (c.mips - standard) as u8,
                tiles: tail_tiles as u32,
                start: 0,
            };
        }
        if let Some(s) = shape.as_mut() {
            *s = [l.block[0] * block, l.block[1] * block, l.block[2]];
        }
        if let Some(n) = count.as_mut() {
            let slices = if tail { l.slices.min(2) } else { l.slices };
            let wanted = (standard * slices).saturating_sub(first).min(*n);
            if !out.is_null() {
                for i in 0..wanted {
                    let (slice, mip) = ((first + i) / standard, (first + i) % standard);
                    let m = &l.mips[mip as usize];
                    let entry = if tail && slice > 0 {
                        SubresourceTiling {
                            width: 0,
                            height: 0,
                            depth: 0,
                            start: 0,
                        }
                    } else {
                        SubresourceTiling {
                            width: m.pitch / l.block[0],
                            height: (m.rows / l.block[1]) as u16,
                            depth: 1,
                            start: ((u64::from(slice) * l.slice_size + m.offset) / TILE) as u32,
                        }
                    };
                    out.add(i as usize).write(entry);
                }
            }
            *n = wanted;
        }
    }
    S_OK
}

/// Slot 0x30 `(plane, mip, x, y, z or slice, sample, bit position)`: an
/// element's byte offset from the texture's start. The title reads
/// texels of a 128x4 `4KB_S` texture on the CPU through it (title+0x5151c90);
/// an error returned as an offset made it read wild memory. 0 if outside.
extern "system" fn element_offset(
    this: This,
    plane: u32,
    mip: u32,
    x: u64,
    y: u32,
    z: u32,
    _sample: u32,
    bit: *mut u32,
) -> u64 {
    if !bit.is_null() {
        // SAFETY: the caller passes room for the bit position or null.
        unsafe { bit.write(0) };
    }
    let Some(p) = computer(this).texture.planes.get(plane as usize) else {
        return 0;
    };
    let x = u32::try_from(x).unwrap_or(u32::MAX);
    match p.layout.offset(mip, x, y, z) {
        Ok(at) => p.offset + at,
        Err(e) => {
            tracing::warn!(?e, mip, x, y, z, "texel outside its texture");
            0
        }
    }
}

/// Slot 0x18: `GetResourceSizeBytes()`.
extern "system" fn resource_size_bytes(this: This) -> u64 {
    computer(this).texture.size
}

/// Slot 0x20: `GetResourceBaseAlignmentBytes()`.
extern "system" fn resource_base_alignment(this: This) -> u64 {
    computer(this).texture.alignment
}

/// Slot 0x28: `GetMipLevelOffsetBytes(Plane, MipLevel)`.
extern "system" fn mip_level_offset(this: This, plane: u32, mip: u32) -> u64 {
    let Some(p) = computer(this).texture.planes.get(plane as usize) else {
        return 0;
    };
    let Some(m) = p.layout.mips.get(mip as usize) else {
        return 0;
    };
    let mip_off = match m.tail {
        Some(t) if !p.layout.thick => u64::from(t.offset),
        _ => m.offset,
    };
    p.offset + mip_off
}

/// Slot 0x60: `IsTiledResource()` (`xg_xs.h:840`, returns `bool`).
extern "system" fn is_tiled_resource(this: This) -> bool {
    const LAYOUT_64KB_UNDEFINED_SWIZZLE: u32 = 2;
    computer(this).desc.layout == LAYOUT_64KB_UNDEFINED_SWIZZLE
}

/// Slots the title does not call: coordinates, view descriptor, swizzle info.
extern "system" fn unused(this: This) -> HRESULT {
    tracing::warn!(desc = ?computer(this).desc, "unimplemented texture computer method");
    E_NOTIMPL
}

/// 14 virtual methods per `XGTextureAddressComputer` (`xg_xs.h:727-848`).
#[repr(C)]
struct Vtable([*const (); 14]);
// SAFETY: function addresses only.
unsafe impl Sync for Vtable {}

static VTABLE: Vtable = Vtable([
    add_ref as *const (),                 // 0x00 AddRef
    release as *const (),                 // 0x08 Release
    layout as *const (),                  // 0x10 GetResourceLayout
    resource_size_bytes as *const (),     // 0x18 GetResourceSizeBytes
    resource_base_alignment as *const (), // 0x20 GetResourceBaseAlignmentBytes
    mip_level_offset as *const (),        // 0x28 GetMipLevelOffsetBytes
    element_offset as *const (),          // 0x30 GetTexelElementOffsetBytes
    unused as *const (),                  // 0x38 GetTexelCoordinate
    copy_into as *const (),               // 0x40 CopyIntoSubresource
    copy_from as *const (),               // 0x48 CopyFromSubresource
    tiling as *const (),                  // 0x50 GetResourceTiling
    unused as *const (),                  // 0x58 GetTextureViewDescriptor
    is_tiled_resource as *const (),       // 0x60 IsTiledResource
    unused as *const (),                  // 0x68 GetSwizzlePatternInfo
]);

pub(crate) fn module() -> HleModule {
    HleModule {
        name: "xg",
        dlls: &["xg_xs.dll"],
        exports: vec![
            hle_export!(XGCreateTextureComputer),
            hle_export!(XGSuggestSwizzleMode),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(width: u64, height: u32, mips: u16, format: u32, layout: u32) -> ResourceDesc {
        ResourceDesc {
            dimension: 3,
            alignment: 0,
            width,
            height,
            depth_or_array: 1,
            mips,
            format,
            samples: 1,
            quality: 0,
            layout,
            flags: 0,
        }
    }

    /// Method `index` of a computer, as the title calls it.
    fn slot<T: Copy>(c: This, index: usize) -> T {
        // SAFETY: the computer is live; T is the slot's function type.
        unsafe { std::mem::transmute_copy(&(*c).vtable.0[index]) }
    }

    fn create(d: &ResourceDesc) -> This {
        let mut c = std::ptr::null_mut();
        assert_eq!(XGCreateTextureComputer(d, &mut c), S_OK);
        c
    }

    /// Element offsets: linear rows of 1 KiB (128 elements), 0 outside.
    #[test]
    fn element_offsets_follow_the_layout() {
        type Offset = extern "system" fn(This, u32, u32, u64, u32, u32, u32, *mut u32) -> u64;
        let c = create(&desc(128, 4, 1, 11, 0x100));
        let offset: Offset = slot(c, 6);
        let mut bit = 7;
        assert_eq!(offset(c, 0, 0, 3, 1, 0, 0, &mut bit), (128 + 3) * 8);
        assert_eq!(bit, 0);
        assert_eq!(offset(c, 0, 0, 128, 0, 0, 0, std::ptr::null_mut()), 0);
        slot::<extern "system" fn(This) -> u32>(c, 1)(c);
    }

    #[test]
    fn suggestions_follow_the_use() {
        let mut out = 0;
        let mut d = desc(256, 256, 1, 98, 0);
        assert_eq!(XGSuggestSwizzleMode(&d, 0, &mut out), S_OK);
        assert_eq!(out, 0x105);
        d.flags = ALLOW_RENDER_TARGET;
        XGSuggestSwizzleMode(&d, 0, &mut out);
        assert_eq!(out, 0x11b);
        d.flags = ALLOW_DEPTH_STENCIL;
        XGSuggestSwizzleMode(&d, 0, &mut out);
        assert_eq!(out, 0x118);
        // A 1D texture has no tiled layout, whatever its use.
        d.dimension = TEXTURE_1D;
        XGSuggestSwizzleMode(&d, 0, &mut out);
        assert_eq!(out, 0x100);
    }

    /// The title's calls: layout, then a copy in and out of each mip.
    #[test]
    fn layout_and_copies_through_the_object() {
        // 256x256 BC7, full chain, 64 KiB display mode.
        let c = create(&desc(256, 256, 0, 98, 0x10a));
        // SAFETY: plain data, filled by the call.
        let mut l: Box<LayoutOut> = Box::new(unsafe { std::mem::zeroed() });
        let get: extern "system" fn(This, *mut LayoutOut) -> HRESULT = slot(c, 2);
        assert_eq!(get(c, &mut *l), S_OK);
        assert_eq!(
            (l.size, l.alignment, l.mips, l.planes),
            (131072, 65536, 9, 1)
        );
        let p = &l.plane[0];
        assert_eq!((p.standard_mips, p.tail_mips, p.element_bytes), (1, 8, 16));
        assert_eq!((p.mips[0].offset, p.mips[0].pitch_bytes), (65536, 1024));
        assert_eq!((p.mips[1].offset, p.mips[1].in_tail), (32768, 1));

        type Into = extern "system" fn(This, *mut u8, u32, u32, *const u8, u32, u32) -> HRESULT;
        type From =
            extern "system" fn(This, *mut u8, u32, u32, u32, *const u8, u32, u32) -> HRESULT;
        let (into, from): (Into, From) = (slot(c, 8), slot(c, 9));
        let mut tiled = vec![0u8; l.size as usize];
        for mip in 0..9u32 {
            let m = &p.mips[mip as usize];
            let row = m.elements[0] * 16;
            let rows: Vec<u8> = (0..row * m.elements[1]).map(|i| (i ^ mip) as u8).collect();
            assert_eq!(
                into(c, tiled.as_mut_ptr(), 0, mip, rows.as_ptr(), row, 0),
                S_OK
            );
            let mut back = vec![0u8; rows.len()];
            assert_eq!(
                from(c, back.as_mut_ptr(), 0, mip, 0, tiled.as_ptr(), row, 0),
                S_OK
            );
            assert_eq!(back, rows, "mip {mip}");
        }
        // 0x50: one tile for the tail, one for mip 0.
        type Tiling = extern "system" fn(
            This,
            *mut u32,
            *mut PackedMips,
            *mut [u32; 3],
            *mut u32,
            u32,
            *mut SubresourceTiling,
        ) -> HRESULT;
        let tiling: Tiling = slot(c, 10);
        let (mut tiles, mut shape, mut n) = (0, [0; 3], 4);
        let mut packed = PackedMips {
            standard: 0,
            packed: 0,
            tiles: 0,
            start: 0,
        };
        let mut subs: Vec<SubresourceTiling> = (0..4)
            .map(|_| SubresourceTiling {
                width: 0,
                height: 0,
                depth: 0,
                start: 0,
            })
            .collect();
        assert_eq!(
            tiling(
                c,
                &mut tiles,
                &mut packed,
                &mut shape,
                &mut n,
                0,
                subs.as_mut_ptr()
            ),
            S_OK
        );
        assert_eq!(
            (tiles, packed.standard, packed.packed, packed.tiles),
            (2, 1, 8, 1)
        );
        assert_eq!((shape, n, subs[0].start), ([256, 256, 1], 1, 1));
        assert_eq!(release(c), 0);

        // 4 KiB modes have no 64 KiB tiling.
        let null = std::ptr::null_mut::<u8>();
        let c = create(&desc(256, 256, 0, 98, 0x105));
        assert_eq!(
            tiling(
                c,
                &mut tiles,
                null.cast(),
                null.cast(),
                null.cast(),
                0,
                null.cast()
            ),
            E_FAIL
        );
        assert_eq!(release(c), 0);
    }

    #[test]
    fn bad_descriptions_fail() {
        let mut c = std::ptr::null_mut();
        let bad = desc(256, 256, 1, 0, 0x10a);
        assert_eq!(XGCreateTextureComputer(&bad, &mut c), E_INVALIDARG);
        let bad = desc(256, 256, 1, 28, 0x103);
        assert_eq!(XGCreateTextureComputer(&bad, &mut c), E_INVALIDARG);
        assert!(module().exports.len() == 2);
    }

    #[test]
    fn vtable_size_alignment_mip_offset_and_is_tiled_match_layout() {
        let c = create(&desc(256, 256, 4, 28, 0x109)); // SW_64KB_S
        let mut l = std::mem::MaybeUninit::<LayoutOut>::zeroed();
        assert_eq!(layout(c, l.as_mut_ptr()), S_OK);
        // SAFETY: initialized by `layout`.
        let l = unsafe { l.assume_init() };
        assert_eq!(resource_size_bytes(c), l.size);
        assert_eq!(resource_base_alignment(c), l.alignment);
        for mip in 0..4u32 {
            assert_eq!(mip_level_offset(c, 0, mip), l.plane[0].mips[mip as usize].offset);
        }
        assert!(!is_tiled_resource(c));
        // Null `out` pointer in `tiling` must safely return count without crashing.
        let mut count = 4u32;
        assert_eq!(
            tiling(
                c,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut count,
                0,
                std::ptr::null_mut(),
            ),
            S_OK
        );
        assert_eq!(release(c), 0);

        // Layout 2 (`XG_TEXTURE_LAYOUT_64KB_UNDEFINED_SWIZZLE`) is tiled.
        let tiled = create(&desc(256, 256, 4, 28, 2));
        assert!(is_tiled_resource(tiled));
        assert_eq!(release(tiled), 0);
    }

    /// Cross-validates `xg.rs` + `victory-formats::texture` byte-for-byte against
    /// a PC `xg_xs.dll` supplied via `VICTORY_GDKX_TOOLS_DIR` (skips when unset).
    #[cfg(windows)]
    #[test]
    fn matches_pc_xg_xs_dll_layout_and_tiling_if_present() {
        use std::ffi::c_void;
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::FreeLibrary;
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

        let Some(dir) = std::env::var_os("VICTORY_GDKX_TOOLS_DIR") else {
            return;
        };
        let dll = std::path::PathBuf::from(dir).join("xg_xs.dll");
        if !dll.is_file() {
            return;
        }
        let wide: Vec<u16> = dll.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: loading an explicit PC reference DLL path from env var.
        let module = unsafe { LoadLibraryW(wide.as_ptr()) };
        assert!(!module.is_null(), "failed to load {}", dll.display());

        type CreateFn = extern "system" fn(*const ResourceDesc, *mut *mut c_void) -> HRESULT;
        type SetHwFn = extern "system" fn(u32) -> HRESULT;
        // SAFETY: standard exported C symbols in `xg_xs.dll`.
        let create_sdk: CreateFn = unsafe {
            let p = GetProcAddress(module, c"XGCreateTextureComputer".as_ptr().cast())
                .expect("XGCreateTextureComputer export");
            std::mem::transmute(p)
        };
        let set_hw: Option<SetHwFn> = unsafe {
            GetProcAddress(module, c"XGSetHardwareVersion".as_ptr().cast())
                .map(|p| std::mem::transmute(p))
        };
        if let Some(f) = set_hw {
            let _ = f(5); // XG_HARDWARE_VERSION_SCARLETT_ANACONDA
        }

        // Deny compression/HTILE metadata planes so plane 0 data matches 1:1.
        const DENY_META: u32 = 0x30_0000 | 0x8000_0000;
        let cases: &[(u64, u32, u16, u16, u32, u32)] = &[
            // (w, h, slices, mips, format, layout)
            (13, 17, 1, 5, 28, 0x105),     // R8G8B8A8 odd SW_4KB_S
            (37, 53, 3, 6, 28, 0x109),     // R8G8B8A8 array SW_64KB_S
            (127, 65, 1, 7, 10, 0x10a),    // R16G16B16A16_FLOAT SW_64KB_D
            (256, 256, 1, 9, 71, 0x105),   // BC1_UNORM SW_4KB_S with mip tail
            (513, 257, 2, 10, 77, 0x109),  // BC3_UNORM odd array SW_64KB_S
            (132, 68, 3, 6, 98, 0x109),    // BC7_UNORM odd array SW_64KB_S
            (256, 128, 1, 8, 98, 0x106),   // BC7_UNORM SW_4KB_D
            (64, 64, 1, 7, 98, 0x101),     // BC7_UNORM SW_256B_S
            (64, 64, 1, 7, 28, 0x102),     // R8G8B8A8 SW_256B_D
            (128, 128, 1, 1, 40, 0x109),   // D32_FLOAT SW_64KB_S
            (128, 128, 1, 1, 20, 0x109),   // D32_FLOAT_S8X24_UINT SW_64KB_S
            (128, 128, 1, 1, 45, 0x109),   // D24_UNORM_S8_UINT SW_64KB_S
        ];

        for &(w, h, slices, mips, format, layout_val) in cases {
            let mut d = desc(w, h, mips, format, layout_val);
            d.depth_or_array = slices;
            d.flags = DENY_META;

            let mut sdk_c = std::ptr::null_mut();
            let hr = create_sdk(&d, &mut sdk_c);
            assert_eq!(hr, S_OK, "SDK failed on {d:?}");

            // Our HLE computer ignores meta deny flags; clear flags for ours.
            let mut our_d = d;
            our_d.flags = 0;
            let our_c = create(&our_d);

            // SAFETY: 14-slot XGTextureAddressComputer vtable (`xg_xs.h:727-848`).
            let vt = unsafe { *sdk_c.cast::<*const *const c_void>() };
            type LayoutFn = extern "system" fn(*mut c_void, *mut LayoutOut) -> HRESULT;
            type ReleaseFn = extern "system" fn(*mut c_void) -> u32;
            type CopyIntoFn = extern "system" fn(*mut c_void, *mut u8, u32, u32, *const u8, u32, u32) -> HRESULT;
            type CopyFromFn = extern "system" fn(*mut c_void, *mut u8, u32, u32, u32, *const u8, u32, u32) -> HRESULT;
            let sdk_layout: LayoutFn = unsafe { std::mem::transmute(*vt.add(2)) };
            let sdk_release: ReleaseFn = unsafe { std::mem::transmute(*vt.add(1)) };
            let sdk_copy_into: CopyIntoFn = unsafe { std::mem::transmute(*vt.add(8)) };
            let sdk_copy_from: CopyFromFn = unsafe { std::mem::transmute(*vt.add(9)) };

            let mut l_sdk = std::mem::MaybeUninit::<LayoutOut>::zeroed();
            let mut l_our = std::mem::MaybeUninit::<LayoutOut>::zeroed();
            assert_eq!(sdk_layout(sdk_c, l_sdk.as_mut_ptr()), S_OK);
            assert_eq!(layout(our_c, l_our.as_mut_ptr()), S_OK);
            // SAFETY: initialized by `GetResourceLayout`.
            let (l_sdk, l_our) = unsafe { (l_sdk.assume_init(), l_our.assume_init()) };

            assert_eq!(l_our.size, l_sdk.size, "size mismatch for {d:?}");
            assert_eq!(l_our.alignment, l_sdk.alignment, "alignment mismatch for {d:?}");
            assert_eq!(l_our.mips, l_sdk.mips, "mips mismatch for {d:?}");
            assert_eq!(l_our.planes, l_sdk.planes, "planes mismatch for {d:?}");
            for p in 0..l_sdk.planes as usize {
                let ps = &l_sdk.plane[p];
                let po = &l_our.plane[p];
                assert_eq!(po.offset, ps.offset, "plane {p} offset for {d:?}");
                assert_eq!(po.size, ps.size, "plane {p} size for {d:?}");
                assert_eq!(po.slice_size, ps.slice_size, "plane {p} slice_size for {d:?}");
                assert_eq!(po.element_bytes, ps.element_bytes, "plane {p} bpe for {d:?}");
                for m in 0..l_sdk.mips as usize {
                    let ms = &ps.mips[m];
                    let mo = &po.mips[m];
                    assert_eq!(mo.offset, ms.offset, "p{p} m{m} offset for {d:?}");
                    assert_eq!(mo.size, ms.size, "p{p} m{m} size for {d:?}");
                    assert_eq!(mo.pitch_bytes, ms.pitch_bytes, "p{p} m{m} pitch for {d:?}");
                    assert_eq!(mo.in_tail, ms.in_tail, "p{p} m{m} in_tail for {d:?}");
                    assert_eq!(mo.tail_position, ms.tail_position, "p{p} m{m} tail_pos for {d:?}");
                }
            }

            // Byte-for-byte CopyIntoSubresource / CopyFromSubresource comparison
            // across every mip and slice for single-plane color/BC formats.
            if l_sdk.planes == 1 {
                let mut tiled_sdk = vec![0u8; l_sdk.size as usize];
                let mut tiled_our = vec![0u8; l_our.size as usize];
                let bpe = l_sdk.plane[0].element_bytes as usize;
                for slice in 0..u32::from(slices) {
                    for mip in 0..l_sdk.mips {
                        let sub = slice * l_sdk.mips + mip;
                        let m = &l_sdk.plane[0].mips[mip as usize];
                        let row_bytes = m.elements[0] as usize * bpe;
                        let row_pitch = row_bytes + 16;
                        let slice_bytes = row_pitch * m.elements[1] as usize;
                        let src: Vec<u8> = (0..slice_bytes)
                            .map(|i| ((i * 29 + sub as usize * 17 + 5) & 0xff) as u8)
                            .collect();
                        assert_eq!(
                            sdk_copy_into(
                                sdk_c,
                                tiled_sdk.as_mut_ptr(),
                                0,
                                sub,
                                src.as_ptr(),
                                row_pitch as u32,
                                slice_bytes as u32
                            ),
                            S_OK,
                            "sdk copy_into sub={sub} {d:?}"
                        );
                        assert_eq!(
                            copy_into(
                                our_c,
                                tiled_our.as_mut_ptr(),
                                0,
                                sub,
                                src.as_ptr(),
                                row_pitch as u32,
                                slice_bytes as u32
                            ),
                            S_OK,
                            "our copy_into sub={sub} {d:?}"
                        );
                        // Detile SDK's tiled buffer using our `copy_from` and confirm exact match.
                        let mut detiled_our = vec![0xeeu8; slice_bytes];
                        let mut detiled_sdk = vec![0xeeu8; slice_bytes];
                        assert_eq!(
                            copy_from(
                                our_c,
                                detiled_our.as_mut_ptr(),
                                0,
                                sub,
                                0,
                                tiled_sdk.as_ptr(),
                                row_pitch as u32,
                                slice_bytes as u32
                            ),
                            S_OK
                        );
                        assert_eq!(
                            sdk_copy_from(
                                sdk_c,
                                detiled_sdk.as_mut_ptr(),
                                0,
                                sub,
                                0,
                                tiled_sdk.as_ptr(),
                                row_pitch as u32,
                                slice_bytes as u32
                            ),
                            S_OK
                        );
                        for y in 0..m.elements[1] as usize {
                            let off = y * row_pitch;
                            assert_eq!(
                                &detiled_our[off..off + row_bytes],
                                &detiled_sdk[off..off + row_bytes],
                                "detile mismatch sub={sub} y={y} {d:?}"
                            );
                        }
                    }
                }
                assert_eq!(tiled_our, tiled_sdk, "tiled bytes mismatch for {d:?}");
            }

            let _ = sdk_release(sdk_c);
            assert_eq!(release(our_c), 0);
        }

        // SAFETY: freeing the DLL loaded above.
        unsafe { FreeLibrary(module) };
    }
}
