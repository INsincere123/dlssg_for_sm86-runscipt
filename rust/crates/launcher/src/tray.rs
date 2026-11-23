//! 托盘图标：交接后窗口可以隐藏，托盘仍能显示状态并提供入口。
//!
//! 托盘进程本身是 `asInvoker`，"恢复原状"只负责再拉起一次提权子进程。

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

/// 用户在托盘里点了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Show,
    Logs,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    show: MenuItem,
    logs: MenuItem,
    quit: MenuItem,
}

impl Tray {
    /// 创建托盘；失败（例如无桌面会话）时返回 `None`，界面照常工作。
    pub fn new(title: &str) -> Option<Self> {
        let show = MenuItem::new("显示窗口", true, None);
        let logs = MenuItem::new("打开日志目录", true, None);
        let quit = MenuItem::new("退出", true, None);
        let menu = Menu::new();
        let separator = PredefinedMenuItem::separator();
        menu.append(&show).ok()?;
        menu.append(&logs).ok()?;
        menu.append(&separator).ok()?;
        menu.append(&quit).ok()?;

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(title)
            .with_icon(make_icon())
            .build()
            .ok()?;
        Some(Self {
            _icon: icon,
            show,
            logs,
            quit,
        })
    }

    /// 取出一次菜单动作（每次界面帧调用一次）。
    pub fn poll(&self) -> Option<Action> {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id;
            if id == self.show.id() {
                return Some(Action::Show);
            }
            if id == self.logs.id() {
                return Some(Action::Logs);
            }
            if id == self.quit.id() {
                return Some(Action::Quit);
            }
        }
        None
    }

    /// 更新悬停提示（用于显示当前状态）。
    pub fn set_tooltip(&self, text: &str) {
        let _ = self._icon.set_tooltip(Some(text));
    }
}

/// 32×32 的圆角方块图标：深色底 + 蓝色渐变，无外部资源依赖。
fn make_icon() -> tray_icon::Icon {
    tray_icon::Icon::from_rgba(icon_pixels(), 32, 32).expect("图标数据合法")
}

/// 32×32 RGBA 像素：深色底 + 蓝色发光中心，无外部资源依赖。
fn icon_pixels() -> Vec<u8> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let index = ((y * SIZE + x) * 4) as usize;
            let cx = x as f32 - 15.5;
            let cy = y as f32 - 15.5;
            let radius = (cx * cx + cy * cy).sqrt();
            // 圆角范围外透明
            if x < 2 || y < 2 || x > SIZE - 3 || y > SIZE - 3 {
                continue;
            }
            let glow = (1.0 - radius / 18.0).clamp(0.0, 1.0);
            rgba[index] = (40.0 + 60.0 * glow) as u8;
            rgba[index + 1] = (110.0 + 90.0 * glow) as u8;
            rgba[index + 2] = (200.0 + 55.0 * glow) as u8;
            rgba[index + 3] = 255;
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_pixels_are_rgba_and_center_is_opaque() {
        // `Icon` 不暴露宽高，改为直接校验我们生成的像素缓冲。
        let rgba = icon_pixels();
        assert_eq!(rgba.len(), (32 * 32 * 4) as usize);
        let center = ((16 * 32 + 16) * 4) as usize;
        assert_eq!(rgba[center + 3], 255, "中心必须不透明");
        assert_eq!(&rgba[..4], &[0, 0, 0, 0], "左上角（圆角外）必须透明");
    }

    #[test]
    fn actions_are_distinct() {
        assert_ne!(Action::Show, Action::Quit);
        assert_ne!(Action::Logs, Action::Show);
    }
}
