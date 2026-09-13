// ═══════════════════════════════════════════════════════════
// saori.rs（新規ファイル）
// ═══════════════════════════════════════════════════════════

use std::collections::HashMap;
use std::ffi::c_long;
use std::path::Path;
use encoding_rs::SHIFT_JIS;  // ★追加
use winapi::shared::minwindef::{HGLOBAL, HMODULE};
use winapi::um::libloaderapi::{FreeLibrary, GetProcAddress, LoadLibraryW};
use winapi::um::winbase::{GlobalAlloc, GlobalFree, GlobalSize, GMEM_FIXED};

type LoadFn    = unsafe extern "C" fn(HGLOBAL, c_long) -> i32;
type RequestFn = unsafe extern "C" fn(HGLOBAL, *mut c_long) -> HGLOBAL;
type UnloadFn  = unsafe extern "C" fn() -> i32;

pub struct SaoriDll {
    handle:     HMODULE,
    load_fn:    LoadFn,
    request_fn: RequestFn,
    unload_fn:  UnloadFn,
}

// HMODULEはSendでないのでラッパーで対処
unsafe impl Send for SaoriDll {}

// ↓ 追加。unload メソッドは削除
impl Drop for SaoriDll {
    fn drop(&mut self) {
        unsafe {
            (self.unload_fn)();
            FreeLibrary(self.handle);
        }
    }
}
impl SaoriDll {
    /// DLLをロードして関数ポインタを取得
    pub fn load(dll_path: &Path) -> Result<Self, String> {
        let wide: Vec<u16> = dll_path.to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            return Err(format!("LoadLibrary失敗: {:?}", dll_path));
        }

        let load_fn = unsafe {
            GetProcAddress(handle, b"load\0".as_ptr() as *const i8)
        };
        let request_fn = unsafe {
            GetProcAddress(handle, b"request\0".as_ptr() as *const i8)
        };
        let unload_fn = unsafe {
            GetProcAddress(handle, b"unload\0".as_ptr() as *const i8)
        };

        if load_fn.is_null() || request_fn.is_null() || unload_fn.is_null() {
            unsafe { FreeLibrary(handle); }
            return Err(format!("SAORI関数が見つかりません: {:?}", dll_path));
        }

        let dll = SaoriDll {
            handle,
            load_fn:    unsafe { std::mem::transmute(load_fn) },
            request_fn: unsafe { std::mem::transmute(request_fn) },
            unload_fn:  unsafe { std::mem::transmute(unload_fn) },
        };

   // saori.rs — SaoriDll::load 内、"SAORIのload呼び出し" のブロックを置き換え

// SAORIのload呼び出し。
// SAORI/1.0仕様では、load時に「自分（SAORI DLL）が置かれている
// ディレクトリパス」を受け取れることが前提の実装が多い
// （設定ファイルや辞書ファイルを自分のディレクトリから探すため）。
// 以前は空文字列を渡していたため、そうしたSAORIが自分の作業ディレクトリを
// 見失っていた。


// SAORI/1.0は歴史的にShift_JISでパスを受け取る実装が主流のため、
// まずCP932でエンコードする。CP932に載らない文字を含むパスの場合は
// UTF-8にフォールバックする（何も渡さないよりは安全なため）。
let saori_dir = dll_path.parent().unwrap_or_else(|| Path::new("."));
let init_bytes = build_load_payload(saori_dir);

unsafe {
    let mem = GlobalAlloc(GMEM_FIXED, init_bytes.len());
    std::ptr::copy_nonoverlapping(init_bytes.as_ptr(), mem as *mut u8, init_bytes.len());
    (dll.load_fn)(mem, (init_bytes.len() - 1) as c_long);
}

        Ok(dll)
    }

    /// SAORIリクエストを送って結果を返す
    pub fn request(&self, args: &[String]) -> HashMap<String, String> {
        let req = build_request(args);
        let bytes = req.as_bytes();

        let response = unsafe {
            let mem = GlobalAlloc(GMEM_FIXED, bytes.len() + 1);
            let ptr = mem as *mut u8;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
            *ptr.add(bytes.len()) = 0;

            let mut len = bytes.len() as c_long;
            let res_mem = (self.request_fn)(mem, &mut len);
            // requestの引数memはrequest_fn内でGlobalFreeされる

            if res_mem.is_null() {
                return HashMap::new();
            }
            // SAORI DLL（外部プラグイン）が書き込んだlenは信用しない。
            // OSがres_memについて把握している実際の確保サイズ（GlobalSize）を
            // 真の上限として使い、lenをその範囲内にクランプする。
            let alloc_size = GlobalSize(res_mem);
            let safe_len = clamp_response_len(len, alloc_size);
            let res_bytes = std::slice::from_raw_parts(res_mem as *const u8, safe_len);
            let s = String::from_utf8_lossy(res_bytes).to_string();
            GlobalFree(res_mem);
            s
        };

        parse_response(&response)
    }

}

/// SAORI DLLが`request_fn`経由で報告してきた応答長`len`を検証する。
///
/// 壊れた/悪意あるSAORI DLLがlenに負の値や実際のバッファより大きい値を
/// 返すと、そのままusizeへキャストして`from_raw_parts`に渡した場合に
/// 範囲外メモリ読み取り（クラッシュ・情報漏洩）につながる。
/// `alloc_size`にはOSが把握している実際の確保サイズ（GlobalSize）を渡し、
/// 常にその範囲内に収まる長さを返す。
fn clamp_response_len(len: c_long, alloc_size: usize) -> usize {
    if len < 0 {
        0
    } else {
        (len as usize).min(alloc_size)
    }
}

/// SAORI/1.0リクエスト文字列を組み立てる
fn build_request(args: &[String]) -> String {
    let mut req = "EXECUTE SAORI/1.0\r\nCharset: UTF-8\r\n".to_string();
    for (i, arg) in args.iter().enumerate() {
        req.push_str(&format!("Argument{}: {}\r\n", i, arg));
    }
    req.push_str("\r\n");
    req
}
/// SAORIのload呼び出しに渡すバイト列を組み立てる。
/// CP932でエンコードできればCP932、できなければUTF-8にフォールバックする。
/// 戻り値はNUL終端を含む。呼び出し側はlenとして (戻り値.len() - 1) を使うこと。
///
/// SAORI/1.0の実装には、自分のディレクトリパスの末尾に区切り文字が
/// 付いている前提で「dir + ファイル名」と素朴に連結するものがある。
/// dll_path.parent()は末尾の区切り文字を含まないため、ここで明示的に付与する。
/// 呼び出し元が既に区切り文字付きのパスを渡してきた場合は二重に付けない。
fn build_load_payload(dir: &Path) -> Vec<u8> {
    let mut dir_str = dir.to_string_lossy().into_owned();
    if !dir_str.ends_with('\\') && !dir_str.ends_with('/') {
        dir_str.push('\\');
    }
    let (encoded, _, had_errors) = SHIFT_JIS.encode(&dir_str);
    let mut bytes: Vec<u8> = if had_errors {
        dir_str.as_bytes().to_vec()
    } else {
        encoded.into_owned()
    };
    bytes.push(0);
    bytes
}

/// SAORI/1.0レスポンスをパースしてValue*, Result等を返す
/// 返り値のMapキー: "0"=Value0相当(Result), "1"=Value1, "2"=Value2...
fn parse_response(response: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in response.lines() {
        if let Some(rest) = line.strip_prefix("Value") {
            if let Some((n, val)) = rest.split_once(": ") {
                map.insert(n.to_string(), val.trim().to_string());
            }
        } else if let Some(val) = line.strip_prefix("Result: ") {
            // ResultはValue0相当として"0"キーに入れる（未設定時のみ）
            map.entry("0".to_string()).or_insert_with(|| val.trim().to_string());
        }
    }
    map
}
// saori.rs 末尾に追加
#[cfg(test)]
mod tests {
    use super::*;

#[test]
fn test_build_load_payload_ascii_path_is_shift_jis_encoded() {
    let dir = Path::new("C:/ghost/minato");
    let payload = build_load_payload(dir);
    assert_eq!(payload, b"C:/ghost/minato\\\0");
}

#[test]
fn test_build_load_payload_japanese_path_uses_shift_jis() {
    let dir = Path::new("C:/ゴースト/湊");
    let payload = build_load_payload(dir);
    assert_eq!(*payload.last().unwrap(), 0);
    let (decoded, _, had_errors) = SHIFT_JIS.decode(&payload[..payload.len() - 1]);
    assert!(!had_errors, "CP932への変換に失敗している: {:?}", payload);
    assert_eq!(decoded, "C:/ゴースト/湊\\");
}

#[test]
fn test_build_load_payload_length_excludes_nul_terminator() {
    let dir = Path::new("C:/test");
    let payload = build_load_payload(dir);
    let len_for_saori = payload.len() - 1;
    assert_eq!(len_for_saori, "C:/test\\".len());
}

#[test]
fn test_clamp_response_len_rejects_negative_len() {
    // 壊れた/悪意あるSAORIがlenに負の値を返してもusize化で巨大値にならない
    assert_eq!(clamp_response_len(-1, 1024), 0);
    assert_eq!(clamp_response_len(c_long::MIN, 1024), 0);
}

#[test]
fn test_clamp_response_len_caps_at_actual_allocation_size() {
    // lenが実際のGlobalAlloc確保サイズより大きい値を主張していても、
    // 確保サイズを超えて読まないようクランプされる
    assert_eq!(clamp_response_len(1_000_000, 16), 16);
}

#[test]
fn test_clamp_response_len_passes_through_when_within_bounds() {
    assert_eq!(clamp_response_len(10, 16), 10);
    assert_eq!(clamp_response_len(0, 16), 0);
}

#[test]
fn test_build_load_payload_does_not_double_separator() {
    // 呼び出し元が既に区切り文字付きのパス文字列を渡してきても、
    // 区切り文字が二重に付かないこと
    let dir = Path::new("C:/ghost/minato/");
    let payload = build_load_payload(dir);
    let (decoded, _, _) = SHIFT_JIS.decode(&payload[..payload.len() - 1]);
    assert_eq!(decoded, "C:/ghost/minato/", "区切り文字が二重に付いている");
}
}