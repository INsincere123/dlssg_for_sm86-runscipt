//! 托盘图标：窗口隐藏后仍能从托盘回到界面，并提供日志与退出入口。
//!
//! 交互按最常规的桌面习惯来：**左键单击 = 打开窗口**，**右键 = 菜单**
//! （`menu_on_left_click(false)`，右键菜单由系统负责弹）。托盘进程本身是 `asInvoker`。

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

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
            // 左键不弹菜单（留给"打开窗口"），右键照旧由系统弹出菜单。
            .with_menu_on_left_click(false)
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

    /// 取出一次动作（每帧调用一次）：菜单点击优先，其次是左键单击/双击图标。
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
        // 左键按下/抬起都会来事件，只认抬起，避免一次点击触发两次。
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            let left_click = matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            );
            if left_click {
                return Some(Action::Show);
            }
        }
        None
    }

    /// 更新悬停提示（用于显示当前状态）。
    pub fn set_tooltip(&self, text: &str) {
        let _ = self._icon.set_tooltip(Some(text));
    }
}

fn make_icon() -> tray_icon::Icon {
    let icon = crate::icon::app_icon();
    tray_icon::Icon::from_rgba(icon.rgba, icon.width, icon.height).expect("图标数据合法")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_distinct() {
        assert_ne!(Action::Show, Action::Quit);
        assert_ne!(Action::Logs, Action::Show);
    }
}
