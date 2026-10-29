//! Pictures decoded by the Windows Imaging Component, with whatever codecs
//! are installed (HEIF Image Extensions, AV1 Video Extension, Raw Image
//! Extension, …). Scaled down while decoding, so huge RAW files stay fast.

use super::{Bitmap, Source};
#[cfg(windows)]
use crate::i18n::t as tr;

/// What to install from the Microsoft Store to read `ext` files.
#[cfg(windows)]
fn missing_codec(ext: &str) -> String {
    let app = match ext {
        "heic" | "heif" | "hif" => "HEIF Image Extensions + HEVC Video Extensions",
        "avif" => "AV1 Video Extension",
        "jxl" => "JPEG XL Image Extension",
        "jxr" | "wdp" | "hdp" => return tr("Windows has no codec for this format.").to_owned(),
        _ => "Raw Image Extension",
    };
    crate::trf!(
        "Windows has no codec for this format. Install {app} from the Microsoft Store.",
        app = app
    )
}

#[cfg(windows)]
pub(super) fn supported() -> bool {
    true
}

#[cfg(not(windows))]
pub(super) fn supported() -> bool {
    false
}

#[cfg(not(windows))]
pub(super) fn decode(
    _src: Source,
    _ext: &str,
    _max_side: u32,
) -> Result<(Bitmap, Option<u16>), String> {
    Err(crate::i18n::t("This format can only be shown on Windows.").to_owned())
}

#[cfg(not(windows))]
pub(super) fn dimensions(_path: &std::path::Path) -> Option<[u32; 2]> {
    None
}

#[cfg(windows)]
pub(super) use imp::{decode, dimensions};

#[cfg(windows)]
mod imp {
    use windows::core::{Interface, HRESULT, HSTRING};
    use windows::Win32::Foundation::{
        GENERIC_READ, WINCODEC_ERR_COMPONENTNOTFOUND, WINCODEC_ERR_UNKNOWNIMAGEFORMAT,
    };
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_MetadataFormatHeif, GUID_WICPixelFormat32bppRGBA,
        IWICBitmapDecoder, IWICBitmapFrameDecode, IWICBitmapSource, IWICImagingFactory,
        IWICMetadataBlockReader, IWICStream, WICBitmapDitherTypeNone,
        WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
        WICHeifOrientation,
    };
    use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::Variant::{VT_UI1, VT_UI2, VT_UI4};

    use super::{missing_codec, Bitmap, Source};

    /// Media Foundation has no decoder for the picture's video codec (a
    /// HEIC without HEVC Video Extensions).
    const MF_E_TOPO_CODEC_NOT_FOUND: HRESULT = HRESULT(0xC00D_5212_u32 as i32);

    /// COM for this thread while it lives.
    struct Com(bool);

    impl Com {
        fn new() -> Self {
            // SAFETY: paired with CoUninitialize on drop when it succeeded.
            Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: CoInitializeEx succeeded on this thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// An opened picture. The stream (and the bytes it reads) must outlive
    /// the decoder.
    struct Opened {
        factory: IWICImagingFactory,
        frame: IWICBitmapFrameDecode,
        _decoder: IWICBitmapDecoder,
        _stream: Option<IWICStream>,
    }

    fn open(src: Source) -> windows::core::Result<Opened> {
        // SAFETY: plain COM calls on interfaces created here; `src`'s bytes
        // outlive the stream reading them (both live in this call's caller).
        unsafe {
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let (decoder, stream) = match src {
                Source::File(path) => (
                    factory.CreateDecoderFromFilename(
                        &HSTRING::from(path.as_os_str()),
                        None,
                        GENERIC_READ,
                        WICDecodeMetadataCacheOnDemand,
                    )?,
                    None,
                ),
                Source::Bytes(bytes) => {
                    let stream = factory.CreateStream()?;
                    stream.InitializeFromMemory(bytes)?;
                    let decoder = factory.CreateDecoderFromStream(
                        &stream,
                        std::ptr::null(),
                        WICDecodeMetadataCacheOnDemand,
                    )?;
                    (decoder, Some(stream))
                }
            };
            let frame = decoder.GetFrame(0)?;
            Ok(Opened {
                factory,
                frame,
                _decoder: decoder,
                _stream: stream,
            })
        }
    }

    /// The turn the codec reports but doesn't apply (HEIF's `irot`/`imir`),
    /// as an EXIF orientation: by its query path, else from the HEIF
    /// metadata block.
    fn orientation(frame: &IWICBitmapFrameDecode) -> Option<u16> {
        // SAFETY: COM calls on the frame; PROPVARIANTs are read by their
        // type tag and cleared after.
        unsafe {
            let mut value = PROPVARIANT::default();
            let by_path = frame.GetMetadataQueryReader().is_ok_and(|reader| {
                reader
                    .GetMetadataByName(&HSTRING::from("/heifProps/Orientation"), &mut value)
                    .is_ok()
            });
            if !by_path {
                let blocks: IWICMetadataBlockReader = frame.cast().ok()?;
                let heif = (0..blocks.GetCount().ok()?)
                    .filter_map(|i| blocks.GetReaderByIndex(i).ok())
                    .find(|r| r.GetMetadataFormat().ok() == Some(GUID_MetadataFormatHeif))?;
                let mut id = PROPVARIANT::default();
                (*id.Anonymous.Anonymous).vt = VT_UI2;
                (*id.Anonymous.Anonymous).Anonymous.uiVal = WICHeifOrientation.0 as u16;
                heif.GetValue(std::ptr::null(), &id, &mut value).ok()?;
            }
            let inner = &value.Anonymous.Anonymous;
            let o = match inner.vt {
                VT_UI1 => Some(u16::from(inner.Anonymous.bVal)),
                VT_UI2 => Some(inner.Anonymous.uiVal),
                VT_UI4 => u16::try_from(inner.Anonymous.ulVal).ok(),
                _ => None,
            };
            let _ = PropVariantClear(&mut value);
            o.filter(|o| (1..=8).contains(o))
        }
    }

    fn error(e: windows::core::Error, ext: &str) -> String {
        let code = e.code();
        if code == WINCODEC_ERR_COMPONENTNOTFOUND
            || code == WINCODEC_ERR_UNKNOWNIMAGEFORMAT
            || code == MF_E_TOPO_CODEC_NOT_FOUND
        {
            missing_codec(ext)
        } else {
            crate::trf!("Can't read this image: {e}", e = e.message())
        }
    }

    /// Decode `src` with its long side at most `max_side`. Also the
    /// orientation still to apply, if the codec reports one.
    pub(in crate::picture) fn decode(
        src: Source,
        ext: &str,
        max_side: u32,
    ) -> Result<(Bitmap, Option<u16>), String> {
        let _com = Com::new();
        let opened = open(src).map_err(|e| error(e, ext))?;
        let o = orientation(&opened.frame);
        let bitmap = pixels(&opened, max_side).map_err(|e| error(e, ext))?;
        Ok((bitmap, o))
    }

    /// The frame scaled down to `max_side` and converted to RGBA.
    fn pixels(opened: &Opened, max_side: u32) -> windows::core::Result<Bitmap> {
        // SAFETY: COM calls on the interfaces opened above; the buffer is as
        // large as the stride times the height.
        unsafe {
            let (mut w, mut h) = (0u32, 0u32);
            opened.frame.GetSize(&mut w, &mut h)?;
            if w == 0 || h == 0 {
                return Err(WINCODEC_ERR_UNKNOWNIMAGEFORMAT.into());
            }
            let mut source: IWICBitmapSource = opened.frame.cast()?;
            let max_side = max_side.max(1);
            let (sw, sh) = if w.max(h) > max_side {
                let scale = f64::from(max_side) / f64::from(w.max(h));
                (
                    ((f64::from(w) * scale).round() as u32).max(1),
                    ((f64::from(h) * scale).round() as u32).max(1),
                )
            } else {
                (w, h)
            };
            if (sw, sh) != (w, h) {
                let scaler = opened.factory.CreateBitmapScaler()?;
                scaler.Initialize(&source, sw, sh, WICBitmapInterpolationModeFant)?;
                source = scaler.cast()?;
            }
            let converter = opened.factory.CreateFormatConverter()?;
            converter.Initialize(
                &source,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )?;
            let stride = sw * 4;
            let mut rgba = vec![0u8; (stride * sh) as usize];
            converter.CopyPixels(std::ptr::null(), stride, &mut rgba)?;
            Ok(Bitmap {
                width: sw,
                height: sh,
                rgba,
                full: [w, h],
            })
        }
    }

    /// The picture's size as it is shown, without decoding it.
    pub(in crate::picture) fn dimensions(path: &std::path::Path) -> Option<[u32; 2]> {
        let _com = Com::new();
        let opened = open(Source::File(path)).ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        // SAFETY: a COM call on the frame opened above.
        unsafe { opened.frame.GetSize(&mut w, &mut h) }.ok()?;
        let turned = orientation(&opened.frame)
            .or_else(|| crate::exif_info::orientation(path))
            .is_some_and(super::super::swaps_sides);
        Some(if turned { [h, w] } else { [w, h] })
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// A 2 × 1 BMP: a blue and a red pixel (BMP rows are BGR, bottom up).
    fn bmp() -> Vec<u8> {
        let mut b = Vec::new();
        let pixels: [u8; 8] = [255, 0, 0, 0, 0, 255, 0, 0];
        let size = 14 + 40 + pixels.len() as u32;
        b.extend(b"BM");
        b.extend(size.to_le_bytes());
        b.extend([0u8; 4]);
        b.extend(54u32.to_le_bytes());
        b.extend(40u32.to_le_bytes());
        b.extend(2i32.to_le_bytes());
        b.extend(1i32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(24u16.to_le_bytes());
        b.extend([0u8; 24]);
        b.extend(pixels);
        b
    }

    #[test]
    fn built_in_codecs_decode() {
        let data = bmp();
        let (b, o) = decode(Source::Bytes(&data), "bmp", 100).unwrap();
        assert_eq!((b.width, b.height, b.full), (2, 1, [2, 1]));
        assert_eq!(&b.rgba, &[0, 0, 255, 255, 255, 0, 0, 255]);
        assert_eq!(o, None);
        // Scaled down while decoding.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.bmp");
        std::fs::write(&path, &data).unwrap();
        let (b, _) = decode(Source::File(&path), "bmp", 1).unwrap();
        assert_eq!((b.width, b.height, b.full), (1, 1, [2, 1]));
        assert_eq!(dimensions(&path), Some([2, 1]));
        assert!(decode(Source::Bytes(b"nothing"), "heic", 100).is_err());
    }

    /// Needs a picture whose codec comes from the Microsoft Store (HEIC,
    /// AVIF, a RAW file …): set OX_TEST_PICTURE to its path.
    #[test]
    #[ignore]
    fn store_codecs_decode() {
        let path = std::env::var("OX_TEST_PICTURE").expect("OX_TEST_PICTURE");
        let path = std::path::Path::new(&path);
        let ext = path.extension().unwrap().to_string_lossy().to_lowercase();
        let (raw, codec_turn) = decode(Source::File(path), &ext, 512).unwrap();
        let exif_turn = crate::exif_info::orientation(path);
        println!(
            "codec: {} x {} (of {:?}), its orientation {codec_turn:?}, EXIF {exif_turn:?}",
            raw.width, raw.height, raw.full
        );
        let b = crate::picture::decode(Source::File(path), &ext, 512).unwrap();
        println!("shown: {} x {} (of {:?})", b.width, b.height, b.full);
        assert!(b.width.max(b.height) <= 512);
        assert_eq!(b.rgba.len(), (b.width * b.height * 4) as usize);
        assert_eq!(crate::picture::dimensions(path, &ext), Some(b.full));
    }
}
