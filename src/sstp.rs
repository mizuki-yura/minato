use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// SSTPサーバ（SSP本体）のデフォルトアドレス
const DEFAULT_SSTP_ADDR: &str = "127.0.0.1:9801";

/// 接続確立を待つ上限。ループバック接続は通常ミリ秒未満で確立するか
/// 即座にRSTが返るため、これより長くかかる場合はSSP側が応答不能な
/// 状態にあると判断して打ち切る。
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
/// 送受信それぞれの上限。
const IO_TIMEOUT: Duration = Duration::from_millis(500);

/// SHIORIのrequest処理中（STATEのMutexを握ったまま）に同期でSSTPへ問い合わせる。
///
/// 注意: この関数はSTATEロック保持中に呼ばれるため、ここでブロックしている間、
/// unload/他のrequest等のDLLエクスポートは全てロック待ちになる。さらにSSPの実装に
/// よってはSSTPサーバとSHIORIのrequest呼び出しが同一スレッドで処理される場合があり、
/// その場合はここでの待機がSSP自体のフリーズとして観測される。
/// 各タイムアウトを短く設定しているのはその影響を最小化するためであり、
/// 根本的な解消（非同期化）ではない。
/// 頻繁に呼ぶ・応答を急がない用途では、OnGotVirtualTimeで使っている
/// \![get,property,...] の非同期パターン（投げて後続イベントで受け取る）を優先すること。
pub fn get_property(prop_name: &str) -> String {
    sstp_execute_get_property(prop_name, DEFAULT_SSTP_ADDR).unwrap_or_default()
}

fn sstp_execute_get_property(prop_name: &str, addr: &str) -> Option<String> {
    let request = format!(
        "EXECUTE SSTP/1.1\r\nCommand: GetProperty\r\nReference0: {}\r\nSender: minato\r\nCharset: UTF-8\r\n\r\n",
        prop_name
    );

    let sock_addr: SocketAddr = addr.parse().ok()?;
    let mut stream = match TcpStream::connect_timeout(&sock_addr, CONNECT_TIMEOUT) {
        Ok(s) => s,
        Err(_e) => {
            append_log!(format!("sstp: connect失敗/タイムアウト: {}", addr));
            return None;
        }
    };
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok()?;

    if stream.write_all(request.as_bytes()).is_err() {
        append_log!("sstp: write失敗/タイムアウト");
        return None;
    }

    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        append_log!("sstp: read失敗/タイムアウト");
        return None;
    }

    let first_line = response.lines().next()?;
    if !first_line.contains("200") {
        return None;
    }

    let data = response.split("\r\n\r\n").nth(1)?;
    let value = data.trim_end_matches(|c| c == '\r' || c == '\n').to_string();

    if value.is_empty() { None } else { Some(value) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;
    use std::time::Instant;

    /// 何もlistenしていないポートに対しては即座に接続拒否され、
    /// CONNECT_TIMEOUTを待たずに素早くNoneが返ることの確認。
    #[test]
    fn test_connection_refused_returns_quickly() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind失敗");
        let port = listener.local_addr().unwrap().port();
        drop(listener); // ポートを閉じる。以降このポートへの接続は即RSTされる

        let addr = format!("127.0.0.1:{}", port);
        let start = Instant::now();
        let result = sstp_execute_get_property("name", &addr);
        let elapsed = start.elapsed();

        assert!(result.is_none());
        assert!(elapsed < Duration::from_secs(1), "接続拒否の判定が遅すぎる: {:?}", elapsed);
    }

    /// 接続は受け付けるが応答を返さないサーバに対して、
    /// IO_TIMEOUTで頭打ちになりNoneが返ることの確認（read_timeoutの検証）。
    #[test]
    fn test_unresponsive_server_times_out_within_bound() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind失敗");
        let port = listener.local_addr().unwrap().port();

        let handle = thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                thread::sleep(Duration::from_secs(3));
                drop(stream);
            }
        });

        let addr = format!("127.0.0.1:{}", port);
        let start = Instant::now();
        let result = sstp_execute_get_property("name", &addr);
        let elapsed = start.elapsed();

        assert!(result.is_none());
        // サーバが3秒黙り込んでいても、2秒未満で打ち切られること
        assert!(elapsed < Duration::from_secs(2), "IOタイムアウトが効いていない: {:?}", elapsed);

        let _ = handle.join();
    }

    /// 正常系: SSTP/1.1準拠の200応答を返すダミーサーバに対して、
    /// 値が正しくパースされることの確認（リファクタ後の既存挙動の回帰確認）。
    #[test]
    fn test_successful_response_is_parsed() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind失敗");
        let port = listener.local_addr().unwrap().port();

        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = "SSTP/1.1 200 OK\r\nSender: SSP\r\n\r\n朝霧湊\r\n";
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let addr = format!("127.0.0.1:{}", port);
        let result = sstp_execute_get_property("name", &addr);

        assert_eq!(result.as_deref(), Some("朝霧湊"));

        let _ = handle.join();
    }
}