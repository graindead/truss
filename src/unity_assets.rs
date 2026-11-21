//! Reads sprites out of a Unity game's asset files (format 22, as Unity 2022.3 writes them,
//! without type trees), for showing Clone Hero's own art in the colour editor's preview. The
//! files are only read, on this computer, from the user's own install; nothing is copied out.
//!
//! Only what's needed for that is understood: the object table, and the Texture2D, Sprite and
//! SpriteAtlas objects (laid out as in 2022.3).

use image::RgbaImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const TEXTURE2D: i32 = 28;
const SPRITE: i32 = 213;
const SPRITE_ATLAS: i32 = 687078895;

/// One asset file's objects: (class id, where its bytes are).
pub struct AssetFile {
    path: PathBuf,
    data: Vec<u8>,
    objects: HashMap<i64, (i32, usize, usize)>,
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    start: usize,
}

type R<T> = Result<T, String>;

impl<'a> Reader<'a> {
    fn at(b: &'a [u8], pos: usize) -> Self {
        Reader { b, pos, start: pos }
    }
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let s = self.b.get(self.pos..self.pos + n).ok_or("unexpected end of the asset file")?;
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> R<i16> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> R<i32> {
        Ok(self.u32()? as i32)
    }
    fn i64(&mut self) -> R<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> R<u64> {
        Ok(self.i64()? as u64)
    }
    fn f32(&mut self) -> R<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn be_u32(&mut self) -> R<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn be_i64(&mut self) -> R<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// Pads to four bytes from the object's start.
    fn align(&mut self) {
        let off = self.pos - self.start;
        self.pos += (4 - off % 4) % 4;
    }
    fn len(&mut self) -> R<usize> {
        let n = self.i32()?;
        if n < 0 || n as usize > self.b.len() {
            return Err("bad length in the asset file".into());
        }
        Ok(n as usize)
    }
    fn string(&mut self) -> R<String> {
        let n = self.len()?;
        let s = String::from_utf8_lossy(self.take(n)?).into_owned();
        self.align();
        Ok(s)
    }
    fn cstring(&mut self) -> R<String> {
        let end = self.b[self.pos..].iter().position(|&c| c == 0).ok_or("unterminated string")?;
        let s = String::from_utf8_lossy(&self.b[self.pos..self.pos + end]).into_owned();
        self.pos += end + 1;
        Ok(s)
    }
    fn bytes(&mut self) -> R<&'a [u8]> {
        let n = self.len()?;
        let s = self.take(n)?;
        self.align();
        Ok(s)
    }
    fn pptr(&mut self) -> R<(i32, i64)> {
        Ok((self.i32()?, self.i64()?))
    }
    fn floats<const N: usize>(&mut self) -> R<[f32; N]> {
        let mut v = [0.0; N];
        for x in &mut v {
            *x = self.f32()?;
        }
        Ok(v)
    }
    fn key(&mut self) -> R<([u8; 16], i64)> {
        Ok((self.take(16)?.try_into().unwrap(), self.i64()?))
    }
}

impl AssetFile {
    pub fn open(path: &Path) -> R<Self> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let objects = Self::table(&data).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(AssetFile { path: path.to_path_buf(), data, objects })
    }

    fn table(data: &[u8]) -> R<HashMap<i64, (i32, usize, usize)>> {
        let mut h = Reader::at(data, 0);
        h.be_u32()?;
        h.be_u32()?;
        let version = h.be_u32()?;
        h.be_u32()?;
        if version < 22 {
            return Err(format!("asset format {version} isn't one Truss reads (22 or later)"));
        }
        let big_endian = h.u8()? != 0;
        h.take(3)?;
        if big_endian {
            return Err("big-endian asset files aren't supported".into());
        }
        h.be_u32()?; // metadata size
        h.be_i64()?; // file size
        let data_offset = h.be_i64()? as usize;
        h.be_i64()?;
        let mut r = Reader::at(data, h.pos);
        r.cstring()?; // Unity version
        r.i32()?; // platform
        if r.u8()? != 0 {
            return Err("asset files with type trees aren't supported".into());
        }
        let n_types = r.len()?;
        let mut classes = Vec::with_capacity(n_types);
        for _ in 0..n_types {
            let class = r.i32()?;
            r.u8()?; // stripped
            let script = r.i16()?;
            if class == 114 || script >= 0 {
                r.take(16)?;
            }
            r.take(16)?;
            classes.push(class);
        }
        let n = r.len()?;
        // The object table is aligned to four bytes from the start of the file.
        r.pos += (4 - r.pos % 4) % 4;
        let mut objects = HashMap::with_capacity(n);
        for _ in 0..n {
            let id = r.i64()?;
            let start = r.i64()? as usize;
            let size = r.u32()? as usize;
            let ty = r.i32()?;
            let class = *classes.get(ty as usize).ok_or("bad type index")?;
            objects.insert(id, (class, data_offset + start, size));
        }
        Ok(objects)
    }

    fn reader(&self, id: i64, class: i32) -> R<Reader<'_>> {
        let &(c, at, size) = self.objects.get(&id).ok_or_else(|| format!("object {id} isn't in the file"))?;
        if c != class {
            return Err(format!("object {id} is class {c}, not {class}"));
        }
        if at + size > self.data.len() {
            return Err("object past the end of the file".into());
        }
        Ok(Reader::at(&self.data[..at + size], at))
    }

    fn ids(&self, class: i32) -> impl Iterator<Item = i64> + '_ {
        self.objects.iter().filter(move |(_, o)| o.0 == class).map(|(id, _)| *id)
    }

    /// The texture with this name, if the file has one.
    pub fn texture_named(&self, name: &str) -> Option<i64> {
        self.ids(TEXTURE2D).find(|&id| self.reader(id, TEXTURE2D).and_then(|mut r| r.string()).is_ok_and(|n| n == name))
    }

    /// A texture's first mip, decoded (top row first).
    pub fn texture(&self, id: i64) -> R<RgbaImage> {
        let mut r = self.reader(id, TEXTURE2D)?;
        let name = r.string()?;
        r.i32()?;
        r.u8()?;
        r.u8()?;
        r.align();
        let w = r.i32()? as usize;
        let h = r.i32()? as usize;
        r.u32()?;
        r.i32()?;
        let format = r.i32()?;
        r.i32()?;
        r.u8()?;
        r.u8()?;
        r.u8()?;
        r.align();
        r.string()?;
        r.u8()?;
        r.align();
        r.i32()?;
        r.i32()?;
        r.i32()?;
        r.take(24)?;
        r.i32()?;
        r.i32()?;
        r.bytes()?;
        let inline = r.bytes()?;
        let offset = r.u64()? as usize;
        let size = r.u32()? as usize;
        let stream = r.string()?;
        let owned;
        let bytes: &[u8] = if !inline.is_empty() {
            inline
        } else {
            let file = stream.rsplit('/').next().unwrap_or(&stream);
            let p = self.path.with_file_name(file);
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            f.seek(SeekFrom::Start(offset as u64)).map_err(|e| e.to_string())?;
            let mut buf = vec![0; size];
            f.read_exact(&mut buf).map_err(|e| format!("{name}: {e}"))?;
            owned = buf;
            &owned
        };
        decode(bytes, w, h, format).map_err(|e| format!("{name}: {e}"))
    }
}

/// Decodes a texture's first mip into an image (Unity stores rows bottom first).
fn decode(b: &[u8], w: usize, h: usize, format: i32) -> R<RgbaImage> {
    let mut px = vec![0u32; w * h];
    let blocks = w.div_ceil(4) * h.div_ceil(4);
    let need = |n: usize| if b.len() < n { Err(format!("texture data too short ({} of {n} bytes)", b.len())) } else { Ok(()) };
    match format {
        // BC7, DXT5, DXT1, BC4, BC5
        25 => need(blocks * 16).and_then(|_| texture2ddecoder::decode_bc7(b, w, h, &mut px).map_err(str::to_string))?,
        12 => need(blocks * 16).and_then(|_| texture2ddecoder::decode_bc3(b, w, h, &mut px).map_err(str::to_string))?,
        10 => need(blocks * 8).and_then(|_| texture2ddecoder::decode_bc1(b, w, h, &mut px).map_err(str::to_string))?,
        26 => need(blocks * 8).and_then(|_| texture2ddecoder::decode_bc4(b, w, h, &mut px).map_err(str::to_string))?,
        27 => need(blocks * 16).and_then(|_| texture2ddecoder::decode_bc5(b, w, h, &mut px).map_err(str::to_string))?,
        // RGBA32, ARGB32, RGB24, Alpha8
        4 | 5 | 3 | 1 => {
            let bpp = match format {
                4 | 5 => 4,
                3 => 3,
                _ => 1,
            };
            need(w * h * bpp)?;
            let mut img = RgbaImage::new(w as u32, h as u32);
            for (i, p) in img.pixels_mut().enumerate() {
                let s = &b[i * bpp..i * bpp + bpp];
                p.0 = match format {
                    4 => [s[0], s[1], s[2], s[3]],
                    5 => [s[1], s[2], s[3], s[0]],
                    3 => [s[0], s[1], s[2], 255],
                    _ => [255, 255, 255, s[0]],
                };
            }
            image::imageops::flip_vertical_in_place(&mut img);
            return Ok(img);
        }
        f => return Err(format!("texture format {f} isn't supported")),
    }
    // texture2ddecoder gives BGRA packed in a u32.
    let mut img = RgbaImage::new(w as u32, h as u32);
    for (p, v) in img.pixels_mut().zip(px) {
        let [b0, g, r, a] = v.to_le_bytes();
        p.0 = [r, g, b0, a];
    }
    image::imageops::flip_vertical_in_place(&mut img);
    Ok(img)
}

/// Where a sprite's pixels are: the texture, the rectangle in it (Unity's, from the bottom),
/// where that sits in the sprite, the sprite's size, and how it's packed.
#[derive(Clone, Debug)]
struct Placement {
    texture: i64,
    rect: [f32; 4],
    offset: [f32; 2],
    settings: u32,
}

/// Every sprite in some asset files, by name, cut out of its texture at its full size.
pub struct Sprites {
    files: Vec<AssetFile>,
    /// name → (file, size, placement)
    found: HashMap<String, (usize, [f32; 2], Placement)>,
}

impl Sprites {
    /// Reads the sprite tables of these files (in the game's data folder).
    pub fn open(paths: &[PathBuf]) -> R<Self> {
        let mut files = Vec::new();
        let mut found = HashMap::new();
        for p in paths {
            let f = AssetFile::open(p)?;
            let fi = files.len();
            // Atlases: render data by key.
            let mut atlas: HashMap<i64, HashMap<([u8; 16], i64), Placement>> = HashMap::new();
            for id in f.ids(SPRITE_ATLAS).collect::<Vec<_>>() {
                if let Ok(m) = read_atlas(&f, id) {
                    atlas.insert(id, m);
                }
            }
            // In object order, so two sprites with the same name are told apart the same way
            // every time: the second is "name#2", and so on.
            let mut ids: Vec<i64> = f.ids(SPRITE).collect();
            ids.sort_unstable();
            for id in ids {
                let Ok((mut name, size, key, atlas_id, own)) = read_sprite(&f, id) else { continue };
                let base = name.clone();
                let mut k = 1;
                while found.contains_key(&name) {
                    k += 1;
                    name = format!("{base}#{k}");
                }
                let place = match own {
                    Some(p) => Some(p),
                    None => atlas.get(&atlas_id).and_then(|m| m.get(&key)).cloned(),
                };
                if let Some(p) = place {
                    found.insert(name, (fi, size, p));
                }
            }
            files.push(f);
        }
        Ok(Sprites { files, found })
    }

    /// Cuts out the named sprites (each texture is decoded once).
    pub fn get(&self, names: &[&str]) -> R<HashMap<String, RgbaImage>> {
        let mut textures: HashMap<(usize, i64), RgbaImage> = HashMap::new();
        let mut out = HashMap::new();
        for &n in names {
            let Some((fi, size, p)) = self.found.get(n) else { continue };
            if !textures.contains_key(&(*fi, p.texture)) {
                textures.insert((*fi, p.texture), self.files[*fi].texture(p.texture)?);
            }
            out.insert(n.to_string(), cut(&textures[&(*fi, p.texture)], *size, p));
        }
        Ok(out)
    }
}

/// A sprite at its full size, from its rectangle in the texture.
fn cut(tex: &RgbaImage, size: [f32; 2], p: &Placement) -> RgbaImage {
    let [x, y, w, h] = p.rect.map(|v| v.round() as i64);
    let (tw, th) = (tex.width() as i64, tex.height() as i64);
    // Unity's rectangles count from the bottom; the decoded texture's rows from the top.
    let top = th - (y + h);
    let (x, top) = (x.clamp(0, tw), top.clamp(0, th));
    let (w, h) = (w.min(tw - x).max(1), h.min(th - top).max(1));
    let mut part = image::imageops::crop_imm(tex, x as u32, top as u32, w as u32, h as u32).to_image();
    // Packed turned or flipped (Unity's SpritePackingRotation, bits 2-5 of the settings).
    if p.settings & 1 != 0 {
        part = match (p.settings >> 2) & 0xF {
            1 => image::imageops::flip_horizontal(&part),
            2 => image::imageops::flip_vertical(&part),
            3 => image::imageops::rotate180(&part),
            4 => image::imageops::rotate270(&part),
            _ => part,
        };
    }
    let (sw, sh) = (size[0].round().max(1.0) as u32, size[1].round().max(1.0) as u32);
    let mut full = RgbaImage::new(sw, sh);
    let ox = p.offset[0].round() as i64;
    let oy = sh as i64 - p.offset[1].round() as i64 - part.height() as i64;
    image::imageops::replace(&mut full, &part, ox, oy);
    full
}

type SpriteInfo = (String, [f32; 2], ([u8; 16], i64), i64, Option<Placement>);

fn read_sprite(f: &AssetFile, id: i64) -> R<SpriteInfo> {
    let mut r = f.reader(id, SPRITE)?;
    let name = r.string()?;
    let rect: [f32; 4] = r.floats()?;
    r.floats::<2>()?; // offset
    r.floats::<4>()?; // border
    r.f32()?;
    r.floats::<2>()?; // pivot
    r.u32()?;
    r.u8()?;
    r.align();
    let key = r.key()?;
    let tags = r.len()?;
    for _ in 0..tags {
        r.string()?;
    }
    let (_, atlas) = r.pptr()?;
    let (tex_file, texture) = r.pptr()?;
    if texture == 0 || tex_file != 0 {
        return Ok((name, [rect[2], rect[3]], key, atlas, None));
    }
    // Its own texture: skip to the render data's rectangle.
    r.pptr()?;
    for _ in 0..r.len()? {
        r.pptr()?;
        r.string()?;
    }
    let subs = r.len()?;
    r.take(subs * 48)?;
    r.bytes()?; // index buffer
    r.u32()?;
    let ch = r.len()?;
    r.take(ch * 4)?;
    r.bytes()?; // vertex data
    let bind = r.len()?;
    r.take(bind * 64)?;
    let trect: [f32; 4] = r.floats()?;
    let offset: [f32; 2] = r.floats()?;
    r.floats::<2>()?;
    let settings = r.u32()?;
    Ok((name, [rect[2], rect[3]], key, atlas, Some(Placement { texture, rect: trect, offset, settings })))
}

fn read_atlas(f: &AssetFile, id: i64) -> R<HashMap<([u8; 16], i64), Placement>> {
    let mut r = f.reader(id, SPRITE_ATLAS)?;
    r.string()?;
    let n = r.len()?;
    r.take(n * 12)?;
    for _ in 0..r.len()? {
        r.string()?;
    }
    let n = r.len()?;
    let mut map = HashMap::with_capacity(n);
    for _ in 0..n {
        let key = r.key()?;
        let (tex_file, texture) = r.pptr()?;
        r.pptr()?;
        let rect: [f32; 4] = r.floats()?;
        let offset: [f32; 2] = r.floats()?;
        r.floats::<2>()?;
        r.floats::<4>()?;
        r.f32()?;
        let settings = r.u32()?;
        for _ in 0..r.len()? {
            r.pptr()?;
            r.string()?;
        }
        if tex_file == 0 {
            map.insert(key, Placement { texture, rect, offset, settings });
        }
    }
    Ok(map)
}

/// Clone Hero's data folder ("Clone Hero_Data") in an install folder, if it's one.
pub fn data_folder(install: &Path) -> Option<PathBuf> {
    let d = install.join("Clone Hero_Data");
    d.join("resources.assets").is_file().then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the strike ring pieces out of a real install (TRUSS_CH_DIR) and saves a sheet of
    /// them to TRUSS_CH_SHEET to look at.
    #[test]
    #[ignore]
    fn reads_a_real_install() {
        let Some(dir) = std::env::var_os("TRUSS_CH_DIR") else { return };
        let data = data_folder(Path::new(&dir)).expect("not a Clone Hero folder");
        let t = std::time::Instant::now();
        let s = Sprites::open(&[data.join("resources.assets")]).unwrap();
        eprintln!("{} sprites listed in {:?}", s.found.len(), t.elapsed());
        let names = ["base_1", "cover_1", "half_cover_1", "head_1", "head_light_1", "inner_color_1", "pick_1", "body0001", "Kickbody"];
        let t = std::time::Instant::now();
        let got = s.get(&names).unwrap();
        eprintln!("cut out in {:?}", t.elapsed());
        assert_eq!(got.len(), names.len());
        assert_eq!(got["head_1"].dimensions(), (192, 96));
        if let Some(out) = std::env::var_os("TRUSS_CH_SHEET") {
            let mut sheet = RgbaImage::from_pixel(1400, 300, image::Rgba([30, 30, 36, 255]));
            let mut x = 0;
            for n in &names[..7] {
                image::imageops::overlay(&mut sheet, &got[*n], x, 0);
                x += 200;
            }
            image::imageops::overlay(&mut sheet, &got["body0001"], 0, 110);
            image::imageops::overlay(&mut sheet, &got["Kickbody"], 0, 200);
            sheet.save(PathBuf::from(out)).unwrap();
        }
    }
}
