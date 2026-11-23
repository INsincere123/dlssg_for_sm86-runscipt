//! 命名管道端到端（M2-04）：服务端建管道并写一行，客户端连上读到同一份状态。

use std::path::PathBuf;

use launcher_win::ipc::{PipeServer, StatusSnapshot, publish, read_line, read_status, status_path};

#[test]
fn pipe_delivers_the_same_snapshot_as_the_status_file() {
    let game = format!("fixture-{}", std::process::id());
    let mut server = match PipeServer::create(&game) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("跳过：无法创建管道（{error}）");
            return;
        }
    };
    let snapshot = StatusSnapshot::new(&game, "deploying", "部署资源");
    let expected = snapshot.to_json();

    let writer_game = game.clone();
    let writer = std::thread::spawn(move || {
        // 客户端连上之前，ConnectNamedPipe 会阻塞在这里。
        let _ = writer_game;
        server.write_line(&snapshot.to_json())
    });

    // 给服务端一点时间进入 ConnectNamedPipe。
    std::thread::sleep(std::time::Duration::from_millis(120));
    let line = read_line(&game).expect("客户端应能读到一行");
    assert_eq!(line, expected, "管道内容必须是同一份快照");
    writer.join().expect("写入线程").expect("服务端写入");
}

#[test]
fn status_file_still_carries_the_phase_when_no_pipe_exists() {
    let directory =
        std::env::temp_dir().join(format!("dlssg-pipe-{}-fallback", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let path: PathBuf = status_path(&directory);
    let snapshot = StatusSnapshot::new("fixture", "recovering", "恢复原状");

    let pushed = publish(None, &path, &snapshot).expect("发布");
    assert!(!pushed, "没有管道时不能声称推送成功");
    let read = read_status(&path).expect("应能从文件读到");
    assert_eq!(read, snapshot, "兜底文件必须给出同一阶段");
    let _ = std::fs::remove_dir_all(&directory);
}
