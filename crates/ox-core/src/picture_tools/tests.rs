use super::job::run;
use super::*;
use crate::undo::{now, Bin};
use image::{ImageEncoder, ImageReader, Rgba, RgbaImage};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// A Recycle Bin in a folder: (original path, where it is kept, deletion time).
struct TestBin {
    dir: PathBuf,
    items: Mutex<Vec<(PathBuf, PathBuf, i64)>>,
}

impl TestBin {
    fn new(root: &Path) -> Self {
        let dir = root.join("bin");
        fs::create_dir_all(&dir).unwrap();
        Self {
            dir,
            items: Mutex::new(Vec::new()),
        }
    }

    fn len(&self) -> usize {
        self.items.lock().unwrap().len()
    }
}

impl Bin for TestBin {
    fn delete(&self, paths: &[PathBuf]) -> Result<(), String> {
        let mut items = self.items.lock().unwrap();
        for p in paths {
            let kept = self.dir.join(format!("{}", items.len()));
            fs::rename(p, &kept).map_err(|e| e.to_string())?;
            items.push((p.clone(), kept, now()));
        }
        Ok(())
    }

    fn restore(&self, paths: &[PathBuf], since: i64) -> Result<(), String> {
        let mut items = self.items.lock().unwrap();
        for p in paths {
            let i = items
                .iter()
                .position(|(orig, _, at)| orig == p && *at >= since)
                .ok_or("not in the bin")?;
            let (orig, kept, _) = items.remove(i);
            fs::rename(kept, orig).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
const BLUE: Rgba<u8> = Rgba([0, 0, 255, 255]);

/// A `w`×`h` blue picture with a red top-left pixel, saved as `name`.
fn picture(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
    let mut img = RgbaImage::from_pixel(w, h, BLUE);
    img.put_pixel(0, 0, RED);
    let path = dir.join(name);
    DynamicImage::ImageRgba8(img).save(&path).unwrap();
    path
}

fn open(path: &Path) -> RgbaImage {
    image::open(path).unwrap().to_rgba8()
}

fn format_of(path: &Path) -> ImageFormat {
    ImageReader::open(path)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .format()
        .unwrap()
}

/// Run `task` on `paths` on one thread.
fn go(task: Task, paths: &[PathBuf], bin: &TestBin) -> Summary {
    let job = Job::new(paths.len());
    run(&job, &task, paths, bin, 1, &|| {})
}

fn convert_to(format: OutputFormat, destination: Destination) -> Convert {
    Convert {
        format,
        destination,
        ..Default::default()
    }
}

#[test]
fn supported_by_extension() {
    assert!(is_supported(Path::new("a.JPG")));
    assert!(is_supported(Path::new("a.tiff")));
    assert!(!is_supported(Path::new("a.svg")));
    assert!(!is_supported(Path::new("jpg")));
}

#[test]
fn rotating_turns_the_pixels_and_keeps_an_undoable_original() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    let p = picture(tmp.path(), "a.png", 4, 2);
    let s = go(
        Task::Transform(Transform::RotateRight),
        std::slice::from_ref(&p),
        &bin,
    );
    assert!(s.failed.is_empty(), "{:?}", s.failed);
    assert_eq!(s.replaced.len(), 1);
    assert_eq!(s.replaced[0].original, p);
    assert_eq!(s.replaced[0].new.path, p);
    let img = open(&p);
    assert_eq!(img.dimensions(), (2, 4));
    // Top left went to the top right.
    assert_eq!(*img.get_pixel(1, 0), RED);
    assert_eq!(bin.len(), 1);
    // No temporary files are left.
    let names: Vec<_> = fs::read_dir(tmp.path()).unwrap().collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn every_transform_moves_the_corner() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    for (tf, size, corner) in [
        (Transform::RotateLeft, (2, 4), (0, 3)),
        (Transform::Rotate180, (4, 2), (3, 1)),
        (Transform::FlipHorizontal, (4, 2), (3, 0)),
        (Transform::FlipVertical, (4, 2), (0, 1)),
    ] {
        let p = picture(tmp.path(), "b.bmp", 4, 2);
        let s = go(Task::Transform(tf), std::slice::from_ref(&p), &bin);
        assert!(s.failed.is_empty(), "{tf:?}: {:?}", s.failed);
        let img = open(&p);
        assert_eq!(img.dimensions(), size, "{tf:?}");
        assert_eq!(*img.get_pixel(corner.0, corner.1), RED, "{tf:?}");
        assert_eq!(format_of(&p), ImageFormat::Bmp);
    }
}

/// EXIF data (little-endian TIFF) with the camera make "Ox" and an orientation.
fn exif_with_orientation(orientation: u8) -> Vec<u8> {
    let mut v = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
    v.extend([2, 0]);
    v.extend([0x0f, 0x01, 2, 0, 3, 0, 0, 0, b'O', b'x', 0, 0]);
    v.extend([0x12, 0x01, 3, 0, 1, 0, 0, 0, orientation, 0, 0, 0]);
    v.extend([0, 0, 0, 0]);
    v
}

fn read_exif(path: &Path) -> exif::Exif {
    let mut r = std::io::BufReader::new(fs::File::open(path).unwrap());
    exif::Reader::new().read_from_container(&mut r).unwrap()
}

#[test]
fn rotated_jpeg_is_upright_and_keeps_its_exif() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    let p = tmp.path().join("photo.jpg");
    // Stored 40×20, shown turned right (orientation 6): 20×40.
    let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(40, 20, BLUE));
    let mut out = Vec::new();
    let mut e = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
    e.set_exif_metadata(exif_with_orientation(6)).unwrap();
    img.to_rgb8().write_with_encoder(e).unwrap();
    fs::write(&p, &out).unwrap();
    assert_eq!(preview(&p, &Convert::default()).unwrap().size, (20, 40));

    let s = go(
        Task::Transform(Transform::RotateRight),
        std::slice::from_ref(&p),
        &bin,
    );
    assert!(s.failed.is_empty(), "{:?}", s.failed);
    // Turned right once more from how it was shown, and stored that way.
    assert_eq!(open(&p).dimensions(), (40, 20));
    let exif = read_exif(&p);
    let orientation = exif
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0));
    assert_eq!(orientation, Some(1));
    let make = exif.get_field(exif::Tag::Make, exif::In::PRIMARY).unwrap();
    assert!(
        matches!(&make.value, exif::Value::Ascii(v) if v[0] == b"Ox"),
        "{:?}",
        make.value
    );
}

#[test]
fn resize_sizes() {
    let fit = |width, height, keep_aspect| Resize::Fit {
        width,
        height,
        keep_aspect,
    };
    assert_eq!(
        target_size(4000, 3000, Resize::Percent(50), false),
        (2000, 1500)
    );
    assert_eq!(
        target_size(4000, 3000, fit(800, 800, true), false),
        (800, 600)
    );
    assert_eq!(
        target_size(3000, 4000, fit(800, 800, true), false),
        (600, 800)
    );
    assert_eq!(
        target_size(4000, 3000, fit(800, 800, false), false),
        (800, 800)
    );
    assert_eq!(
        target_size(4000, 3000, Resize::LongestSide(1000), false),
        (1000, 750)
    );
    assert_eq!(
        target_size(1, 1000, Resize::LongestSide(10), false),
        (1, 10)
    );
    // Never bigger, unless asked.
    assert_eq!(
        target_size(400, 300, Resize::Percent(200), false),
        (400, 300)
    );
    assert_eq!(
        target_size(400, 300, Resize::Percent(200), true),
        (800, 600)
    );
    assert_eq!(
        target_size(400, 300, Resize::LongestSide(800), false),
        (400, 300)
    );
    assert_eq!(
        target_size(400, 300, fit(800, 200, false), false),
        (400, 200)
    );
    assert_eq!(
        target_size(400, 300, fit(800, 200, false), true),
        (800, 200)
    );
}

#[test]
fn output_names() {
    assert_eq!(output_name("a.png", ImageFormat::Jpeg, None), "a.jpg");
    assert_eq!(output_name("a.JPEG", ImageFormat::Jpeg, None), "a.JPEG");
    assert_eq!(
        output_name("my.photo.jpg", ImageFormat::WebP, Some("800px")),
        "my.photo (800px).webp"
    );
    assert_eq!(
        output_name("a.tif", ImageFormat::Tiff, Some("50%")),
        "a (50%).tif"
    );
}

#[test]
fn converting_next_to_the_originals() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    let p = picture(tmp.path(), "pic.png", 40, 20);
    let mut c = convert_to(OutputFormat::Jpeg, Destination::Beside);
    let s = go(Task::Convert(c.clone()), std::slice::from_ref(&p), &bin);
    assert!(s.failed.is_empty(), "{:?}", s.failed);
    let jpg = tmp.path().join("pic.jpg");
    assert_eq!(s.created.len(), 1);
    assert_eq!(s.created[0].path, jpg);
    assert_eq!(format_of(&jpg), ImageFormat::Jpeg);
    assert!(p.exists());
    // Again: the name is taken now.
    let s = go(Task::Convert(c.clone()), std::slice::from_ref(&p), &bin);
    assert_eq!(s.created[0].path, tmp.path().join("pic (2).jpg"));

    // Resized, in its own format.
    c.format = OutputFormat::Original;
    c.resize = Some(Resize::LongestSide(10));
    assert_eq!(
        preview(&p, &c),
        Some(Preview {
            size: (40, 20),
            new_size: (10, 5),
            new_name: "pic (10px).png".into(),
        })
    );
    let s = go(Task::Convert(c), std::slice::from_ref(&p), &bin);
    let small = tmp.path().join("pic (10px).png");
    assert_eq!(s.created[0].path, small);
    assert_eq!(open(&small).dimensions(), (10, 5));
    assert_eq!(format_of(&small), ImageFormat::Png);
    assert_eq!(bin.len(), 0);
}

#[test]
fn transparent_pictures_become_white_in_jpeg() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    let p = tmp.path().join("clear.png");
    RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]))
        .save(&p)
        .unwrap();
    let s = go(
        Task::Convert(convert_to(OutputFormat::Jpeg, Destination::Beside)),
        &[p],
        &bin,
    );
    let px = open(&s.created[0].path).get_pixel(4, 4).0;
    assert!(px[0] > 240 && px[1] > 240 && px[2] > 240, "{px:?}");
}

#[test]
fn converting_into_a_folder_makes_it() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    let a = picture(tmp.path(), "a.png", 4, 4);
    let b = picture(tmp.path(), "a.bmp", 4, 4);
    let out = tmp.path().join("out").join("webp");
    let c = convert_to(OutputFormat::WebP, Destination::Folder(out.clone()));
    let s = go(Task::Convert(c), &[a, b], &bin);
    assert!(s.failed.is_empty(), "{:?}", s.failed);
    let made: Vec<PathBuf> = s.created.iter().map(|p| p.path.clone()).collect();
    assert_eq!(made, vec![out.join("a.webp"), out.join("a (2).webp")]);
    assert_eq!(format_of(&made[0]), ImageFormat::WebP);
}

#[test]
fn replacing_the_originals() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    let p = picture(tmp.path(), "pic.png", 40, 20);
    let c = Convert {
        resize: Some(Resize::Percent(50)),
        ..convert_to(OutputFormat::WebP, Destination::Replace)
    };
    let s = go(Task::Convert(c), std::slice::from_ref(&p), &bin);
    assert!(s.failed.is_empty(), "{:?}", s.failed);
    let webp = tmp.path().join("pic.webp");
    assert_eq!(s.replaced[0].original, p);
    assert_eq!(s.replaced[0].new.path, webp);
    assert!(!p.exists());
    assert_eq!(open(&webp).dimensions(), (20, 10));
    assert_eq!(bin.len(), 1);
}

#[test]
fn failures_are_collected_and_the_rest_is_done() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    let good = picture(tmp.path(), "good.png", 4, 2);
    let bad = tmp.path().join("bad.png");
    fs::write(&bad, "not a picture").unwrap();
    let s = go(
        Task::Transform(Transform::Rotate180),
        &[bad.clone(), good],
        &bin,
    );
    assert_eq!(s.replaced.len(), 1);
    assert_eq!(s.failed.len(), 1);
    assert_eq!(s.failed[0].0, bad);
    // The broken file is untouched.
    assert_eq!(fs::read_to_string(&bad).unwrap(), "not a picture");
    assert_eq!(bin.len(), 1);
}

#[test]
fn animated_pictures_are_not_rotated() {
    use image::codecs::gif::GifEncoder;
    use image::Frame;
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    let p = tmp.path().join("anim.gif");
    {
        let mut e = GifEncoder::new(fs::File::create(&p).unwrap());
        e.encode_frames([
            Frame::new(RgbaImage::from_pixel(4, 2, RED)),
            Frame::new(RgbaImage::from_pixel(4, 2, BLUE)),
        ])
        .unwrap();
    }
    let s = go(
        Task::Transform(Transform::RotateLeft),
        std::slice::from_ref(&p),
        &bin,
    );
    assert_eq!(s.failed.len(), 1);
    assert_eq!(bin.len(), 0);
    // Converting takes the first frame.
    let s = go(
        Task::Convert(convert_to(OutputFormat::Png, Destination::Beside)),
        &[p],
        &bin,
    );
    assert!(s.failed.is_empty(), "{:?}", s.failed);
    assert_eq!(*open(&s.created[0].path).get_pixel(0, 0), RED);
}

#[test]
fn cancel_stops_before_the_next_picture() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    let paths: Vec<PathBuf> = (0..5)
        .map(|i| picture(tmp.path(), &format!("{i}.png"), 4, 2))
        .collect();
    let job = Job::new(paths.len());
    let task = Task::Convert(convert_to(OutputFormat::Jpeg, Destination::Beside));
    let s = run(&job, &task, &paths, &bin, 1, &|| job.cancel());
    assert!(s.cancelled);
    assert_eq!(s.created.len(), 1);
    assert_eq!(job.done(), 1);
}

#[test]
fn a_started_job_finishes_on_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let paths: Vec<PathBuf> = (0..6)
        .map(|i| picture(tmp.path(), &format!("{i}.png"), 8, 4))
        .collect();
    let bin = std::sync::Arc::new(TestBin::new(tmp.path()));
    let job = start(Task::Transform(Transform::RotateRight), paths, bin, || {});
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !job.is_finished() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let s = job.take_summary().expect("finished");
    assert_eq!(s.replaced.len(), 6, "{:?}", s.failed);
    assert!(!s.cancelled);
    assert_eq!(job.done(), 6);
    assert!(job.take_summary().is_none());
}

#[cfg(windows)]
#[test]
fn replaced_files_keep_their_created_date() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = TestBin::new(tmp.path());
    let p = picture(tmp.path(), "old.png", 4, 2);
    let then = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    super::job::set_created(&p, then).unwrap();
    go(
        Task::Transform(Transform::FlipVertical),
        std::slice::from_ref(&p),
        &bin,
    );
    let meta = fs::metadata(&p).unwrap();
    assert_eq!(meta.created().unwrap(), then);
    assert!(meta.modified().unwrap() > then);
}
