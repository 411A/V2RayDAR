<p align="center">
  <a href="https://deepwiki.com/411A/V2RayDAR">
    <img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki About V2RayDAR">
  </a>
</p>

<p align="center">
  <strong>🌐 Available in</strong><br>
  <strong><a href="../README.md">English</a></strong>
  • <strong><a href="README.fa.md">فارسی</a></strong>
  • <strong><a href="README.zh-CN.md">简体中文</a></strong>
  • <strong><a href="README.ru.md">Русский</a></strong>
  • <strong><a href="README.fr.md">Français</a></strong>
</p>

<p align="center">
  <img src="../assets/V2RayDAR_logo_v1.png" alt="V2RayDAR logo" width="200" height="200">
</p>

<h1 align="center">V2RayDAR</h1>

<p align="center">
  <em>V2Ray 检测与侦察 — 发音类似 <code>v2ray</code> + <code>radar</code>。</em>
</p>

<p align="center">
  一款快速的 Rust CLI/TUI 工具，用于获取 V2Ray / Clash / Mihomo 订阅源，通过 <code>sing-box</code> 在真实网络中验证配置，对可用配置进行排名，并在本地订阅 URL 上重新发布最佳配置，供 v2rayN / v2rayNG / sing-box / Clash Verge / Mihomo 客户端使用。
</p>

<p align="center">
  📘 <a href="guide.md">阅读详细开发者指南</a>
</p>

## 🖥️ Windows TUI 预览

<p align="center">
  <img src="../assets/Windows_TUI_v0.5.2.png" alt="Windows TUI" width="100%">
</p>

## 🤔 为什么选择 V2RayDAR

- 并行获取任意数量的订阅源。
- 解析原始文本、base64、JSON 和 YAML 格式 — 支持 `vmess`、`vless`、`trojan`、`ss`、`ssr`、`hysteria2`、`hy2`、`tuic` 分享链接。
- **解析 Clash/Mihomo YAML 配置** — 添加 Mihomo 订阅 URL，V2RayDAR 自动提取所有代理条目。
- **双向格式转换** — 在 V2Ray 分享链接和 Clash/Mihomo YAML 代理条目之间互转。
- 通过 `sing-box` 在当前网络中验证每个候选配置（实际加载测试 URL 通过代理）。
- **双格式输出** — 以 V2Ray 分享链接（`/subscription`）**和**完整 Mihomo YAML 配置（`/mihomo.yaml`）提供可用配置，兼容任何客户端。
- 在本地 URL 重新发布最佳可用配置，兼容客户端只需指向一个始终最新的订阅源。
- 通过数据库中先前探测过的配置、网络内桥接配置或 `emergency_config` 在受限网络中存活。
- 可选的局域网共享及令牌保护，方便手机使用同一订阅源。

## 📦 快速安装

将对应操作系统的命令复制到终端。安装脚本自动检测平台，下载最新版本并附带 `sing-box`，完成全部配置。便携模式安装到 `Desktop/V2RayDAR`（若存在桌面目录），否则安装到 `~/V2RayDAR`。用户模式将二进制文件安装到 `~/.local/bin`。

**便携模式**（推荐）— 所有文件在同一目录，使用 `--portable` 运行：
```bash
# Linux
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh

# macOS
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh

# Windows
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
```

**用户安装** — 二进制文件到 `~/.local/bin`，数据在主目录：
```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --user

# Windows
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
# 然后在提示时选择选项 2
```

**Android / Termux：**
```bash
# 先安装 sing-box，然后运行安装脚本
pkg update -y && pkg install -y curl tar sing-box=1.13.13
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh
# Termux 上请始终使用 --no-tui（Termux 终端不支持鼠标输入）
cd V2RayDAR && ./v2raydar --no-tui
```

**手动下载** — 从 [Releases](https://github.com/411A/V2RayDAR/releases/latest) 下载对应操作系统的压缩包，使用 `--portable` 运行。

安装脚本验证 SHA-256 校验和，检测已安装版本并提供更新（保留 `configs.yaml`、`data.db` 和 `v2raydar_data/`），默认无需 sudo。

## 🔰 快速开始

使用上述脚本安装后，运行 `v2raydar`（Windows 上为 `v2raydar.exe`）。首次启动会创建包含预选订阅源的 `configs.yaml`。

1. **等待数据填充。** 应用并行获取订阅源，通过真实网络探测每个配置，并对可用配置进行排名。端点从启动即可用 — 客户端可以立即指向它。
2. **将客户端指向**订阅 URL：

| 客户端 | 端点 |
| --- | --- |
| v2rayN / v2rayNG | `http://127.0.0.1:27141/subscription`（base64） |
| sing-box | `http://127.0.0.1:27141/subscription.txt`（纯文本） |
| Clash Verge / Mihomo | `http://127.0.0.1:27141/mihomo.yaml` |

3. **TUI 控制键：**

| 按键 | 操作 |
| --- | --- |
| `↑` / `↓` 或 `j` / `k` | 导航 |
| `Enter` | 选择 / 切换 / 确认 |
| `Esc` / `Ctrl+H` | 返回 |
| `Space` | 切换订阅开关 |
| `e` | 编辑选中的订阅 |
| `q` | 退出 |
| `:` | 命令模式 — `:q` 退出，`:w` 保存，`:a` 添加，`:d` 删除，`:n` 重命名，`:u` 修改 URL，`:p` 修改优先级 |

4. **更改设置** — 从 TUI 主菜单（Configurations）或直接编辑 `configs.yaml`，更改将在下次刷新时生效。关键设置：`top_n`、`refresh_seconds`、`sharing.enabled`、`probe.mode`。
5. **退出** — 按 `q` 或 `:q`。退出后端点停止服务。

### 运行模式

```bash
v2raydar                # TUI + 本地订阅端点
v2raydar --no-tui       # 无头模式 — 仅端点和日志
v2raydar --once         # 刷新一次，打印结果后退出
v2raydar --portable     # 数据保存在可执行文件旁边
v2raydar --uninstall    # 删除应用数据和防火墙规则
```

Windows 用户将 `v2raydar` 替换为 `v2raydar.exe`。macOS 上首次打开捆绑的 `.app` 后，Gatekeeper 会记住它。

## ⚙️ 默认配置一览

<details>
  <summary>👣 <strong>configs.yaml</strong> — 所有配置键、默认值及用途。详细说明请参阅 <a href="guide.md">开发者指南</a>。</summary>

| 键 | 默认值 | 用途 |
| --- | --- | --- |
| `bind` | `127.0.0.1:27141` | 本地 HTTP 绑定地址，用于 `/subscription`、`/subscription.txt`、`/results` 和 `/health`。 |
| `top_n` | `10` | 发布给客户端的可用配置数量。 |
| `refresh_seconds` | `300` | 自动刷新间隔（秒）；`0` 禁用定时刷新。 |
| `encoded_subscription` | `true` | `/subscription` 返回 base64 编码（兼容 v2rayN / v2rayNG）。 |
| `prioritize_stability` | `true` | 优先重新探测上一轮保存的 Top-N，即使新发现的配置延迟更低也保持其靠前。设为 `false` 则优先选择低延迟的可用配置。 |
| `return_configs_asap` | `false` | 设为 `true` 时，找到可用配置后立即发布到端点，最多 `top_n` 个；早期配置可能不是延迟最低或最稳定的。 |
| `scan_all_configs` | `false` | 设为 `true` 时验证所有加载的配置，而非找到足够可用配置后停止。 |
| `fetch_timeout_ms` | `30000` | 每个源的获取超时。 |
| `fetch_concurrency` | `8` | 并行获取的订阅源数量。 |
| `max_subscription_bytes` | `33554432` | 每个订阅源的大小上限（32 MiB）。 |
| `use_cache_only` | `false` | 跳过实时获取，从数据库加载先前探测过的配置 — 适用于高度受限网络。 |
| `emergency_config` | `null` | 可选的可用分享链接，用于通过 `sing-box` 作为桥接代理在 HTTP 订阅获取失败时使用。 |
| `clean_offlines_after_days` | `7` | 不可达配置从数据库中删除的天数。 |
| `sharing.enabled` | `false` | 允许局域网客户端访问端点。 |
| `sharing.require_token` | `false` | 局域网请求需要 `?token=...`。 |
| `sharing.token` | `null` | 留空则禁用，设为 `true` 自动生成，或提供字符串。 |
| `proxy.enabled` | `false` | 启动持久的 SOCKS5/HTTP 代理进程。 |
| `proxy.port` | `27910` | 混合 SOCKS5/HTTP 代理端口。 |
| `proxy.discoverable` | `false` | 绑定到 0.0.0.0 并添加防火墙规则以允许局域网访问。 |
| `proxy.health_check_url` | `https://www.gstatic.com/generate_204` | 通过代理测试的健康检查 URL。 |
| `proxy.health_check_interval_seconds` | `60` | 代理健康检查间隔（秒）。故障时自动切换。 |
| `probe.mode` | `active` | `active` 使用 `sing-box`；`tcp` 仅用于诊断。 |
| `probe.sing_box_path` | `null` | 可选的 `sing-box` 路径。桌面 `_with_singbox` 构建或 Termux 包路径可设为 `null`。 |
| `probe.connect_timeout_ms` | `5000` | 诊断探测的 TCP 连接超时。 |
| `probe.active_timeout_ms` | `30000` | 活跃模式下的 HTTP 测试超时。 |
| `probe.startup_timeout_ms` | `5000` | 等待临时代理启动的时间。 |
| `probe.concurrency` | `16` | 基础活跃探测并发数。 |
| `probe.batch_size` | `20` | 初始活跃探测批次大小。 |
| `probe.process_concurrency` | `null` | 允许同时运行的 `sing-box` 批处理进程数；为空时自动缩放。 |
| `probe.test_url` | `https://www.gstatic.com/generate_204` | 通过每个候选配置加载的测试 URL。 |
| `probe.accepted_statuses` | `[204, 200]` | 视为成功的 HTTP 状态码。 |
| `probe.download_url` | `null` | 可选的吞吐量测试目标。 |
| `probe.download_bytes_limit` | `1048576` | 每次速度测试的读取上限。 |
| `geoip_db_path` | `null` | 可选的 `GeoLite2-Country.mmdb` 文件路径。为 `null` 时使用内置数据库进行国家检测。 |
| `subscriptions` | _（预选源）_ | `{ name, url, enabled, priority }` 源列表。建议添加自己的源以获得更好的覆盖。 |

</details>

## 🌐 受限网络注意事项

- 在高度受限的网络中，先前探测过的配置存储在数据库中，可通过 `use_cache_only: true` 使用。
- 默认情况下，如果某些 HTTP 订阅 URL 无法连接但有可用配置，应用会使用该配置重试失败的 HTTP 订阅。如果没有可用配置但你有一个可用配置，可以将其添加到 `configs.yaml` 的 `emergency_config` 中，让应用使用它重试失败的 HTTP 订阅获取。

## 📡 将常用客户端指向 V2RayDAR

- **v2rayN（同一台电脑）** — 保持 `bind: 127.0.0.1:27141`，添加 `http://127.0.0.1:27141/subscription` 作为订阅 URL。
- **v2rayNG / 同一 Wi-Fi 的手机** — 绑定到电脑的局域网 IP（如 `192.168.1.23:27141`），开启 `sharing.enabled`，然后在手机上使用 `http://192.168.1.23:27141/subscription`。先从手机访问 `/health` 确认可达性。

完整的客户端配置指南、令牌保护共享和操作系统特定防火墙详情请参阅 [开发者指南](guide.md)。

### 📱 用于应用流量的持久代理

V2RayDAR 可以在订阅端点旁边运行一个持久的 SOCKS5/HTTP 代理。系统上的任何应用 — Telegram、浏览器、curl、Python — 都可以通过它路由流量，无需单独的 VPN 客户端。

**在 `configs.yaml` 中启用：**
```yaml
proxy:
  enabled: true
  port: 27910
  discoverable: false   # true = 局域网访问 + 防火墙规则
```

**本地使用（在运行 V2RayDAR 的设备上）：**
```bash
curl --socks5 127.0.0.1:27910 https://api.ipify.org
```

**局域网使用（同一 Wi-Fi 的手机）：**
1. 设置 `proxy.discoverable: true` — V2RayDAR 会添加防火墙规则并绑定到 `0.0.0.0`。
2. 在 TUI 的 **Current Configuration** 面板 **Network** 部分找到电脑的局域网 IP（或运行 `ipconfig` / `ip addr`）。例如 `192.168.1.2`。
3. **Telegram：** 将 `YOUR_LAN_IP` 替换为你的实际局域网 IP，在手机上打开此 URL：

   ```
   https://t.me/socks?server=YOUR_LAN_IP&port=27910
   ```

   例如，如果你的局域网 IP 是 `192.168.1.2`：
   ```
   https://t.me/socks?server=192.168.1.2&port=27910
   ```

   或手动：Telegram → 设置 → 数据和存储 → 代理设置 → 添加代理：
   - 类型：**SOCKS5** 或 **HTTP**
   - 主机：`YOUR_LAN_IP`（V2RayDAR TUI 面板中显示的 IP）
   - 端口：`27910`

4. **Android 全局代理：** 设置 → WiFi → 长按网络 → 修改 → 高级 → 代理 → 手动 → 服务器：`YOUR_LAN_IP`，端口：`27910`。

## 🤝 贡献

欢迎贡献！可以提交 Issue 报告错误、提出功能请求、问题或建议，也可以提交 Pull Request。任何反馈都非常感谢。

## 🗺 路线图

- [ ] 使用 Tauri 在 TUI 之外添加跨平台 GUI 应用。
- [ ] 从任何网站正文中提取 V2Ray 配置 — 优先从非 JS 密集型网站提取，JS 密集型网站使用 FireCrawl 或 Obscura 作为备选方案。
- [ ] 带密码要求和身份验证的私有端点：当订阅端点是私有的且受密码保护时，用户可以通过国家级可达端点获取配置。

## 👨‍💻 免责声明

本应用按"现状"发布，不提供任何保证。

开发者本身不会创建或分发 V2Ray 兼容配置，也不对用户扫描和连接的 V2Ray 订阅负责。你连接的 V2Ray 服务器所有者可能能够截获你的流量并读取未加密数据。

## ☕️ 联系与捐赠

### 💬 联系方式

<p align="center">
<a href="https://t.me/TechKrakenBot">
  <img src="https://img.shields.io/badge/Telegram-2CA5E0?style=for-the-badge&logo=telegram&logoColor=white" alt="Telegram Bot">
</a>
</p>

### 💎 TON 捐赠

如果你觉得本项目有帮助，可以通过 TON 区块链捐赠支持开发：

```
ton://transfer/TechKraken.ton
```

```
UQCGk4IU5nm6dYWjXTx6vSQVOtKO4LQg3m8cRcq1eQo7vhCl
```
