//! First-page previews of PDF files, rendered by the PDF engine built into
//! Windows (`Windows.Data.Pdf`). Not available on other systems.

use std::io;
use std::path::Path;

pub struct PdfPreview {
    /// The first page as a PNG image.
    pub png: Vec<u8>,
    pub pages: u32,
}

/// Render the first page of `path`, `width` pixels wide.
#[cfg(windows)]
pub fn render_first_page(path: &Path, width: u32) -> io::Result<PdfPreview> {
    imp::render(path, width).map_err(|e| io::Error::other(e.message()))
}

#[cfg(not(windows))]
pub fn render_first_page(_path: &Path, _width: u32) -> io::Result<PdfPreview> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "PDF previews need Windows.",
    ))
}

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use windows::core::{Result, HSTRING};
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{
        Buffer, DataReader, InMemoryRandomAccessStream, InputStreamOptions,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    pub fn render(path: &Path, width: u32) -> Result<super::PdfPreview> {
        // WinRT needs COM on this (background) thread; S_FALSE means it already is.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.join()?;
        let doc = PdfDocument::LoadFromFileAsync(&file)?.join()?;
        let pages = doc.PageCount()?;
        let page = doc.GetPage(0)?;
        let options = PdfPageRenderOptions::new()?;
        options.SetDestinationWidth(width)?;
        // Rendered as PNG into memory, then copied out.
        let stream = InMemoryRandomAccessStream::new()?;
        page.RenderWithOptionsToStreamAsync(&stream, &options)?
            .join()?;
        let size = stream.Size()? as u32;
        let input = stream.GetInputStreamAt(0)?;
        let data = input
            .ReadAsync(&Buffer::Create(size)?, size, InputStreamOptions::None)?
            .join()?;
        let mut png = vec![0u8; data.Length()? as usize];
        DataReader::FromBuffer(&data)?.ReadBytes(&mut png)?;
        Ok(super::PdfPreview { png, pages })
    }
}
