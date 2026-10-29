//! Lossless turning of JPEGs through the EXIF orientation.

use super::exif_turn::{compose, turn_jpeg};
use super::job::run;
use super::*;
use crate::picture::{orient, Bitmap};
use crate::undo::Bin;
use image::{ImageEncoder, Rgb, RgbImage};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// The orientation that does what `tf` does, written out separately from
/// the code under test.
fn as_orientation(tf: Transform) -> u16 {
    match tf {
        Transform::RotateRight => 6,
        Transform::RotateLeft => 8,
        Transform::Rotate180 => 3,
        Transform::FlipHorizontal => 2,
        Transform::FlipVertical => 4,
    }
}

/// A 3 × 2 bitmap whose pixels are all different.
fn bitmap() -> Bitmap {
    Bitmap {
        width: 3,
        height: 2,
        rgba: (0..6u8).flat_map(|i| [i, i, i, 255]).collect(),
        full: [3, 2],
    }
}

fn shown(o: u16) -> Bitmap {
    let mut b = bitmap();
    orient(&mut b, o);
    b
}

#[test]
fn every_orientation_composes_with_every_transform() {
    for o in 1..=8 {
        for tf in Transform::ALL {
            let mut expected = shown(o);
            orient(&mut expected, as_orientation(tf));
            let got = shown(compose(o, tf));
            assert_eq!(
                (got.width, got.height, &got.rgba),
                (expected.width, expected.height, &expected.rgba),
                "orientation {o}, {tf:?}"
            );
        }
    }
}

#[test]
fn composition_table() {
    use Transform::*;
    // Rows: orientation 1–8; columns: right, left, 180°, flip h, flip v.
    let table: [[u16; 5]; 8] = [
        [6, 8, 3, 2, 4],
        [7, 5, 4, 1, 3],
        [8, 6, 1, 4, 2],
        [5, 7, 2, 3, 1],
        [2, 4, 7, 6, 8],
        [3, 1, 8, 5, 7],
        [4, 2, 5, 8, 6],
        [1, 3, 6, 7, 5],
    ];
    for (row, o) in table.iter().zip(1..) {
        for (want, tf) in row.iter().zip([
            RotateRight,
            RotateLeft,
            Rotate180,
            FlipHorizontal,
            FlipVertical,
        ]) {
            assert_eq!(compose(o, tf), *want, "orientation {o}, {tf:?}");
        }
    }
    // Values viewers ignore count as "normal".
    assert_eq!(compose(0, RotateRight), 6);
    assert_eq!(compose(42, FlipVertical), 4);
}

/// EXIF data with the camera make "Ox" and, if given, an orientation (as
/// a SHORT, or as a LONG to make it odd).
fn exif(big_endian: bool, orientation: Option<u16>, long: bool) -> Vec<u8> {
    let u16b = |v: u16| {
        if big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let u32b = |v: u32| {
        if big_endian {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let mut v = if big_endian {
        b"MM\0\x2a".to_vec()
    } else {
        b"II\x2a\0".to_vec()
    };
    v.extend(u32b(8));
    v.extend(u16b(if orientation.is_some() { 2 } else { 1 }));
    // Make, ASCII, 3: "Ox\0" in the value field.
    v.extend(u16b(0x010f));
    v.extend(u16b(2));
    v.extend(u32b(3));
    v.extend([b'O', b'x', 0, 0]);
    if let Some(o) = orientation {
        v.extend(u16b(0x0112));
        if long {
            v.extend(u16b(4));
            v.extend(u32b(1));
            v.extend(u32b(o as u32));
        } else {
            v.extend(u16b(3));
            v.extend(u32b(1));
            v.extend(u16b(o));
            v.extend([0, 0]);
        }
    }
    v.extend(u32b(0));
    v
}

/// A 16 × 8 JPEG, red on the left and blue on the right, with `exif`.
fn jpeg(exif: Option<Vec<u8>>) -> Vec<u8> {
    let img = RgbImage::from_fn(16, 8, |x, _| {
        if x < 8 {
            Rgb([255, 0, 0])
        } else {
            Rgb([0, 0, 255])
        }
    });
    let mut out = Vec::new();
    let mut e = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
    if let Some(x) = exif {
        e.set_exif_metadata(x).unwrap();
    }
    img.write_with_encoder(e).unwrap();
    out
}

fn orientation_read_back(bytes: &[u8]) -> Option<u16> {
    crate::exif_info::orientation_in(bytes)
}

fn make_read_back(bytes: &[u8]) -> Vec<u8> {
    let exif = exif::Reader::new().read_raw(exif_tiff(bytes)).unwrap();
    let make = exif.get_field(exif::Tag::Make, exif::In::PRIMARY).unwrap();
    match &make.value {
        exif::Value::Ascii(v) => v[0].clone(),
        other => panic!("{other:?}"),
    }
}

fn exif_tiff(bytes: &[u8]) -> Vec<u8> {
    super::exif_turn::exif_chunk(bytes).unwrap().to_vec()
}

#[test]
fn the_tag_is_rewritten_in_place() {
    for big_endian in [false, true] {
        let input = jpeg(Some(exif(big_endian, Some(6), false)));
        let (out, o) = turn_jpeg(&input, Transform::RotateRight).unwrap();
        assert_eq!(o, 3);
        assert_eq!(out.len(), input.len());
        let changed: Vec<usize> = (0..out.len()).filter(|&i| out[i] != input[i]).collect();
        // Only the low byte of the value: 6 → 3.
        assert_eq!(changed.len(), 1, "{changed:?}");
        assert_eq!(orientation_read_back(&out), Some(3));
        assert_eq!(make_read_back(&out), b"Ox");
    }
}

/// Where the first scan starts: everything from there on is the pixels.
fn scan(bytes: &[u8]) -> &[u8] {
    let at = bytes.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    &bytes[at..]
}

#[test]
fn a_missing_tag_is_added_to_the_exif_data() {
    for big_endian in [false, true] {
        let input = jpeg(Some(exif(big_endian, None, false)));
        assert_eq!(orientation_read_back(&input), None);
        let (out, o) = turn_jpeg(&input, Transform::RotateLeft).unwrap();
        assert_eq!(o, 8);
        assert_eq!(orientation_read_back(&out), Some(8));
        // The rest of the EXIF data is still there, and the pixels as they were.
        assert_eq!(make_read_back(&out), b"Ox");
        assert_eq!(scan(&out), scan(&input));
        // Turned again: now the tag is there and changed in place.
        let (again, o) = turn_jpeg(&out, Transform::RotateLeft).unwrap();
        assert_eq!(o, 3);
        assert_eq!(again.len(), out.len());
    }
}

#[test]
fn an_exif_segment_is_inserted_when_there_is_none() {
    let input = jpeg(None);
    assert!(super::exif_turn::exif_chunk(&input).is_none());
    let (out, o) = turn_jpeg(&input, Transform::FlipHorizontal).unwrap();
    assert_eq!(o, 2);
    assert_eq!(out.len(), input.len() + 36);
    // Inserted as one piece: the bytes before and after it are the old ones.
    let at = out
        .windows(4)
        .position(|w| w == [0xFF, 0xE1, 0, 34])
        .unwrap();
    assert!(at >= 2);
    assert_eq!(&out[..at], &input[..at]);
    assert_eq!(&out[at + 36..], &input[at..]);
    assert_eq!(orientation_read_back(&out), Some(2));
}

#[test]
fn odd_exif_data_is_not_rewritten() {
    // The orientation as a LONG: decoders ignore it, so would we.
    let input = jpeg(Some(exif(false, Some(6), true)));
    assert!(turn_jpeg(&input, Transform::RotateRight).is_none());
    // Not a JPEG.
    assert!(turn_jpeg(b"\x89PNG\r\n", Transform::RotateRight).is_none());
    // Cut off in the middle of a segment.
    assert!(turn_jpeg(&input[..30], Transform::RotateRight).is_none());
}

struct NoBin(Mutex<Vec<(PathBuf, PathBuf)>>);

impl Bin for NoBin {
    fn delete(&self, paths: &[PathBuf]) -> Result<(), String> {
        for p in paths {
            let kept = p.with_extension("binned");
            fs::rename(p, &kept).map_err(|e| e.to_string())?;
            self.0.lock().unwrap().push((p.clone(), kept));
        }
        Ok(())
    }

    fn restore(&self, _paths: &[PathBuf], _since: i64) -> Result<(), String> {
        Err("no".into())
    }
}

fn turn_file(path: &Path, tf: Transform, how: JpegTurn) -> Summary {
    let bin = NoBin(Mutex::new(Vec::new()));
    let job = Job::new(1);
    let s = run(
        &job,
        &Task::Transform(tf, how),
        std::slice::from_ref(&path.to_path_buf()),
        &bin,
        1,
        &|| {},
    );
    assert!(s.failed.is_empty(), "{:?}", s.failed);
    assert_eq!(s.replaced.len(), 1);
    s
}

#[test]
fn decoded_turned_jpegs_show_the_turned_pixels() {
    for start in [None, Some(6), Some(2)] {
        for tf in Transform::ALL {
            let tmp = tempfile::tempdir().unwrap();
            let p = tmp.path().join("photo.jpg");
            let input = jpeg(Some(exif(false, start, false)));
            fs::write(&p, &input).unwrap();
            let before = super::codec::read(&p).unwrap().img;
            turn_file(&p, tf, JpegTurn::Lossless);
            let bytes = fs::read(&p).unwrap();
            // The pixels weren't re-encoded.
            assert_eq!(scan(&bytes), scan(&input), "{start:?} {tf:?}");
            // Decoded upright (as the preview and thumbnails do), it is turned.
            let after = super::codec::read(&p).unwrap().img;
            assert_eq!(
                after.to_rgb8(),
                tf.apply(&before).to_rgb8(),
                "{start:?} {tf:?}"
            );
        }
    }
}

#[test]
fn reencoding_or_odd_exif_turns_the_pixels() {
    let tmp = tempfile::tempdir().unwrap();
    for (name, exif_data, how) in [
        ("a.jpg", exif(false, None, false), JpegTurn::Reencode),
        ("b.jpg", exif(false, Some(6), true), JpegTurn::Lossless),
    ] {
        let p = tmp.path().join(name);
        let input = jpeg(Some(exif_data));
        fs::write(&p, &input).unwrap();
        turn_file(&p, Transform::RotateRight, how);
        let bytes = fs::read(&p).unwrap();
        assert_ne!(scan(&bytes), scan(&input), "{name}");
        let (w, h) = image::image_dimensions(&p).unwrap();
        assert_eq!((w, h), (8, 16), "{name}");
    }
}

#[test]
fn other_formats_are_still_reencoded() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("a.png");
    DynamicImage::ImageRgb8(RgbImage::new(4, 2))
        .save(&p)
        .unwrap();
    turn_file(&p, Transform::RotateRight, JpegTurn::Lossless);
    assert_eq!(image::image_dimensions(&p).unwrap(), (2, 4));
}
