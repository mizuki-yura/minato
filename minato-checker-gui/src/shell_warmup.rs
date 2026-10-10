// shell_warmup.rs
// フォルダ・ファイル選択画面の初回表示が遅いのは、Windowsがその場でシェル
// （エクスプローラーの部品やシェル拡張）を読み込むため。起動直後に別スレッドで
// 同じ部品を先に読み込んでおき、ボタンを押したときにすぐ開くようにする。
// 失敗しても選択画面が初回に遅いだけなので、エラーはすべて無視する。

/// シェルの事前読み込みを別スレッドで始める。Windows以外では何もしない。
pub fn spawn() {
    #[cfg(windows)]
    {
        let _ = std::thread::Builder::new()
            .name("shell-warmup".to_owned())
            .spawn(warm_up);
    }
}

#[cfg(windows)]
fn warm_up() {
    use std::ptr::null_mut;
    use winapi::shared::winerror::SUCCEEDED;
    use winapi::shared::wtypesbase::CLSCTX_INPROC_SERVER;
    use winapi::um::combaseapi::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize};
    use winapi::um::knownfolders::FOLDERID_Documents;
    use winapi::um::objbase::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use winapi::um::shlobj::SHGetKnownFolderPath;
    use winapi::um::shobjidl::IFileOpenDialog;
    use winapi::um::shobjidl_core::CLSID_FileOpenDialog;
    use winapi::Interface;

    unsafe {
        // rfdと同じ設定（STA）で初期化する
        let hr = CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        if !SUCCEEDED(hr) {
            return;
        }

        // 既知フォルダの解決でshell32まわりを読み込む
        let mut path = null_mut();
        if SUCCEEDED(SHGetKnownFolderPath(&FOLDERID_Documents, 0, null_mut(), &mut path)) {
            CoTaskMemFree(path as _);
        }

        // 選択画面のCOMオブジェクトを一度作って捨て、画面本体のDLLも読み込んでおく
        let mut dialog: *mut IFileOpenDialog = null_mut();
        if SUCCEEDED(CoCreateInstance(
            &CLSID_FileOpenDialog,
            null_mut(),
            CLSCTX_INPROC_SERVER,
            &IFileOpenDialog::uuidof(),
            &mut dialog as *mut _ as *mut _,
        )) && !dialog.is_null()
        {
            (*dialog).Release();
        }

        // 本体のスレッドはドラッグ＆ドロップのためにCOMを初期化したままなので、
        // ここで終了処理をしても読み込んだDLLはプロセスに残る
        CoUninitialize();
    }
}
