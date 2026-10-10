// win_drop.rs
// Windowsでのドラッグ＆ドロップの受け取り。
// winit（0.30）のOLEによるドロップは、ドラッグ元のエクスプローラーからCOMで届く知らせが
// eframeの処理と行き違うと、離した知らせ（DroppedFile）が届かず
// 「ここで離してください」のまま止まることがある。
// そこでWindowsではwinitのドロップを切り、COMを通らない古い仕組み
// （DragAcceptFiles と WM_DROPFILES）で受け取る。
// この仕組みには「ドラッグが窓の上に来た」知らせが無いため、ドラッグ中の表示は出せない。

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winapi::shared::basetsd::{DWORD_PTR, UINT_PTR};
use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::commctrl::{DefSubclassProc, SetWindowSubclass};
use winapi::um::shellapi::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use winapi::um::winuser::WM_DROPFILES;

/// ウィンドウプロシージャからUIへ、ドロップされたパスと再描画の依頼先を渡す
static SINK: Mutex<Option<(Sender<PathBuf>, egui::Context)>> = Mutex::new(None);

/// ウィンドウをドロップ先にする。受け取ったパスはReceiverに届く。失敗したらNone。
pub fn install(window: &impl HasWindowHandle, ctx: &egui::Context) -> Option<Receiver<PathBuf>> {
    let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    let hwnd = handle.hwnd.get() as HWND;
    let (tx, rx) = mpsc::channel();
    *SINK.lock().ok()? = Some((tx, ctx.clone()));
    unsafe {
        if SetWindowSubclass(hwnd, Some(subclass_proc), 1, 0) == 0 {
            return None;
        }
        DragAcceptFiles(hwnd, 1);
    }
    Some(rx)
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: UINT_PTR,
    _data: DWORD_PTR,
) -> LRESULT {
    if msg != WM_DROPFILES {
        return DefSubclassProc(hwnd, msg, wparam, lparam);
    }
    let hdrop = wparam as HDROP;
    // 複数ドロップされても、チェックするのは先頭の1つだけ
    let path = query_file(hdrop, 0);
    DragFinish(hdrop);
    if let (Some(path), Ok(sink)) = (path, SINK.lock()) {
        if let Some((tx, ctx)) = sink.as_ref() {
            let _ = tx.send(path);
            ctx.request_repaint();
        }
    }
    0
}

unsafe fn query_file(hdrop: HDROP, index: UINT) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let len = DragQueryFileW(hdrop, index, std::ptr::null_mut(), 0);
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize + 1];
    let got = DragQueryFileW(hdrop, index, buf.as_mut_ptr(), buf.len() as UINT);
    buf.truncate(got as usize);
    Some(std::ffi::OsString::from_wide(&buf).into())
}
