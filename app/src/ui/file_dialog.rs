//! 原生“另存为”对话框。
//!
//! Windows 下直接调用系统 `comdlg32.dll`，不引入额外 crate；其他平台退化为
//! “没有选择路径”，由调用方保持原有导出逻辑。

use std::path::PathBuf;

/// 弹出保存对话框，返回用户选择的完整路径；取消时返回 `None`。
pub fn save_markdown(suggested_name: &str, initial_dir: Option<&std::path::Path>) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows::save(suggested_name, initial_dir)
    }
    #[cfg(not(windows))]
    {
        let _ = (suggested_name, initial_dir);
        None
    }
}

/// 弹出打开对话框，选择要导入参数的 PDF 或图片；取消时返回 `None`。
pub fn open_document() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows::open_document()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// 用户没有输入扩展名时补上 `.md`。
fn ensure_markdown_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        let mut path = path;
        path.set_extension("md");
        path
    } else {
        path
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    use super::ensure_markdown_extension;

    const OFN_OVERWRITEPROMPT: u32 = 0x0000_0002;
    const OFN_NOCHANGEDIR: u32 = 0x0000_0008;
    const OFN_PATHMUSTEXIST: u32 = 0x0000_0800;
    const OFN_FILEMUSTEXIST: u32 = 0x0000_1000;
    const OFN_EXPLORER: u32 = 0x0008_0000;

    /// `OPENFILENAMEW` 的字段顺序必须与 Windows 头文件完全一致。
    #[repr(C)]
    pub(super) struct OpenFileNameW {
        struct_size: u32,
        owner: *mut c_void,
        instance: *mut c_void,
        filter: *const u16,
        custom_filter: *const u16,
        max_custom_filter: u32,
        filter_index: u32,
        file: *mut u16,
        max_file: u32,
        file_title: *mut u16,
        max_file_title: u32,
        initial_dir: *const u16,
        title: *const u16,
        flags: u32,
        file_offset: u16,
        file_extension: u16,
        default_ext: *const u16,
        custom_data: isize,
        hook: *const c_void,
        template: *const u16,
        reserved: *mut c_void,
        reserved_value: u32,
        flags_ex: u32,
    }

    #[link(name = "comdlg32")]
    extern "system" {
        fn GetSaveFileNameW(dialog: *mut OpenFileNameW) -> i32;
        fn GetOpenFileNameW(dialog: *mut OpenFileNameW) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetActiveWindow() -> *mut c_void;
    }

    pub(super) fn save(suggested_name: &str, initial_dir: Option<&std::path::Path>) -> Option<PathBuf> {
        let mut file: Vec<u16> = suggested_name.encode_utf16().collect();
        file.push(0);
        // 留足空间给用户在对话框里继续输入长文件名。
        file.resize(1024, 0);
        let filter: Vec<u16> = "Markdown 报告 (*.md)\0*.md\0所有文件 (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
        let title = wide("导出选型报告");
        let default_ext = wide("md");
        let initial_dir = initial_dir.map(|dir| {
            dir.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        });
        let owner = unsafe { GetActiveWindow() };

        let mut dialog = OpenFileNameW {
            struct_size: std::mem::size_of::<OpenFileNameW>() as u32,
            owner,
            instance: std::ptr::null_mut(),
            filter: filter.as_ptr(),
            custom_filter: std::ptr::null(),
            max_custom_filter: 0,
            filter_index: 1,
            file: file.as_mut_ptr(),
            max_file: file.len() as u32,
            file_title: std::ptr::null_mut(),
            max_file_title: 0,
            initial_dir: initial_dir
                .as_ref()
                .map(|value| value.as_ptr())
                .unwrap_or(std::ptr::null()),
            title: title.as_ptr(),
            flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_EXPLORER,
            file_offset: 0,
            file_extension: 0,
            default_ext: default_ext.as_ptr(),
            custom_data: 0,
            hook: std::ptr::null(),
            template: std::ptr::null(),
            reserved: std::ptr::null_mut(),
            reserved_value: 0,
            flags_ex: 0,
        };

        let accepted = unsafe { GetSaveFileNameW(&mut dialog) };
        if accepted == 0 {
            return None;
        }

        let length = file
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(file.len());
        let path = PathBuf::from(String::from_utf16_lossy(&file[..length]));
        Some(ensure_markdown_extension(path))
    }

    pub(super) fn open_document() -> Option<PathBuf> {
        let mut file = vec![0_u16; 1024];
        let filter: Vec<u16> = "支持文档 (*.pdf;*.png;*.jpg;*.jpeg;*.bmp;*.tif;*.tiff;*.webp)\0\
            *.pdf;*.png;*.jpg;*.jpeg;*.bmp;*.tif;*.tiff;*.webp\0\
            PDF (*.pdf)\0*.pdf\0\
            图片 (*.png;*.jpg;*.jpeg;*.bmp;*.tif;*.tiff;*.webp)\0\
            *.png;*.jpg;*.jpeg;*.bmp;*.tif;*.tiff;*.webp\0\
            所有文件 (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
        let title = wide("导入电机参数");
        let default_ext = wide("pdf");
        let owner = unsafe { GetActiveWindow() };

        let mut dialog = OpenFileNameW {
            struct_size: std::mem::size_of::<OpenFileNameW>() as u32,
            owner,
            instance: std::ptr::null_mut(),
            filter: filter.as_ptr(),
            custom_filter: std::ptr::null(),
            max_custom_filter: 0,
            filter_index: 1,
            file: file.as_mut_ptr(),
            max_file: file.len() as u32,
            file_title: std::ptr::null_mut(),
            max_file_title: 0,
            initial_dir: std::ptr::null(),
            title: title.as_ptr(),
            flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_EXPLORER,
            file_offset: 0,
            file_extension: 0,
            default_ext: default_ext.as_ptr(),
            custom_data: 0,
            hook: std::ptr::null(),
            template: std::ptr::null(),
            reserved: std::ptr::null_mut(),
            reserved_value: 0,
            flags_ex: 0,
        };

        let accepted = unsafe { GetOpenFileNameW(&mut dialog) };
        if accepted == 0 {
            return None;
        }
        let length = file
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(file.len());
        if length == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&file[..length])))
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_extension_is_filled_with_md() {
        let path = ensure_markdown_extension(PathBuf::from("C:/reports/选型报告"));
        assert_eq!(path.extension().and_then(|value| value.to_str()), Some("md"));
    }

    #[test]
    fn existing_extension_is_kept() {
        let path = ensure_markdown_extension(PathBuf::from("C:/reports/选型报告.txt"));
        assert_eq!(path.extension().and_then(|value| value.to_str()), Some("txt"));
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn dialog_struct_matches_windows_layout() {
        // x64 上 OPENFILENAMEW 固定为 152 字节；不一致会被 GetSaveFileNameW 拒绝。
        assert_eq!(std::mem::size_of::<windows::OpenFileNameW>(), 152);
    }
}
