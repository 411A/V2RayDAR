<div dir="ltr">

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

</div>

<div dir="rtl" align="right">

<p align="center">
  <em>تشخیص و بازشناسی V2Ray &#x200F;— مانند <code>v2ray</code> + <code>radar</code> تلفظ می‌شود.</em>
</p>

<p align="center">
  ابزار سریع CLI/TUI بر پایه Rust که منابع اشتراک &#x200F;V2Ray / Clash / Mihomo &#x200F;را دریافت می‌کند، کانفیگ‌ها را از طریق شبکه واقعی شما با <code>sing-box</code> بررسی می‌کند، بهترین کانفیگ‌ها را رتبه‌بندی کرده و آن‌ها را به صورت محلی منتشر می‌کند تا کلاینت‌های &#x200F;v2rayN / v2rayNG / sing-box / Clash Verge / Mihomo &#x200F;بتوانند از آن استفاده کنند.
</p>

<p align="center">
  &#x200F;📘 <a href="guide.md">راهنمای جامع توسعه‌دهندگان</a>
</p>

## &#x200F;🖥️ پیش‌نمایش TUI ویندوز

<p align="center">
  <img src="../assets/Windows_TUI_v0.5.2.png" alt="Windows TUI" width="100%">
</p>

## &#x200F;🤔 چرا V2RayDAR

- &#x200F;دریافت همزمان اشتراک‌ها از تعداد نامحدودی منابع.
- &#x200F;پشتیبانی از فرمت‌های خام، <code>base64</code>، <code>JSON</code> و <code>YAML</code> &#x200F;— و لینک‌های اشتراک <code>vmess</code>، <code>vless</code>، <code>trojan</code>، <code>ss</code>، <code>ssr</code>، <code>hysteria2</code>، <code>hy2</code>، <code>tuic</code>.
- &#x200F;<strong>پشتیبانی از کانفیگ‌های Clash/Mihomo YAML</strong> &#x200F;— کافیست یک URL اشتراک Mihomo اضافه کنید تا V2RayDAR تمام ورودی‌های پروکسی را به صورت خودکار استخراج کند.
- &#x200F;<strong>تبدیل دوطرفه فرمت</strong> &#x200F;— تبدیل بین لینک‌های اشتراک V2Ray و ورودی‌های پروکسی Clash/Mihomo YAML.
- &#x200F;بررسی هر کانفیگ از طریق شبکه واقعی شما با <code>sing-box</code> &#x200F;(یعنی واقعاً یک URL تست را از مسیر پروکسی بارگذاری می‌کند).
- &#x200F;<strong>خروجی دو فرمت</strong> &#x200F;— کانفیگ‌های کارآمد هم به صورت لینک اشتراک <code>/subscription</code> &#x200F;و هم به صورت کانفیگ کامل Mihomo YAML <code>/mihomo.yaml</code> &#x200F;ارائه می‌شوند.
- &#x200F;انتشار مجدد بهترین کانفیگ‌های کارآمد در یک URL محلی، طوری که هر کلاینت سازگار یک اشتراک همیشه به‌روز داشته باشد.
- &#x200F;کارکرد در شبکه‌های محدود از طریق کانفیگ‌های قبلاً بررسی‌شده در پایگاه داده، پل شبکه داخلی یا <code>emergency_config</code>.
- &#x200F;اشتراک‌گذاری اختیاری در LAN با محافظت اختیاری توکن، مناسب برای استفاده گوشی از همان منبع اشتراک.

## &#x200F;📦 نصب سریع

دستور مخصوص سیستم‌عامل خود را در ترمینال کپی کنید. اسکریپت نصب پلتفرم شما را تشخیص داده، آخرین نسخه را همراه <code>sing-box</code> &#x200F;دانلود کرده و همه چیز را راه‌اندازی می‌کند. حالت پرتابل در <code>Desktop/V2RayDAR</code> &#x200F;(یا در صورت نبود پوشه رومیزی، در <code>~/V2RayDAR</code>) نصب می‌شود. حالت کاربری فایل باینری را در <code>~/.local/bin</code> &#x200F;نصب می‌کند.

&#x200F;<strong>حالت پرتابل</strong> (توصیه‌شده) — همه فایل‌ها در یک پوشه، اجرا با <code>--portable</code>:

<div dir="ltr">

```bash
# Linux
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh

# macOS
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh

# Windows
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
```

</div>

&#x200F;<strong>نصب کاربری</strong> — فایل باینری در <code>~/.local/bin</code>، داده‌ها در پوشه خانه:

<div dir="ltr">

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --user

# Windows
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
# Then choose option 2 when prompted
```

</div>

&#x200F;<strong>Android / Termux:</strong>

<div dir="ltr">

```bash
# Install sing-box, then run the installer
pkg update -y && pkg install -y curl tar sing-box=1.13.13
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh
# Always use --no-tui on Termux (TUI mouse input doesn't work in Termux terminals)
cd V2RayDAR && ./v2raydar --no-tui
```

</div>

&#x200F;<strong>دانلود دستی</strong> — آرشیو مخصوص سیستم‌عامل خود را از <a href="https://github.com/411A/V2RayDAR/releases/latest">Releases</a> دانلود کرده و با <code>--portable</code> اجرا کنید.

&#x200F;اسکریپت نصب هش SHA-256 را بررسی کرده، نصب‌های موجود را شناسایی و پیشنهاد به‌روزرسانی می‌دهد (با حفظ <code>configs.yaml</code>، <code>data.db</code> و <code>v2raydar_data/</code>) و به صورت پیش‌فرض نیازی به sudo ندارد.

## &#x200F;🔰 شروع سریع

&#x200F;پس از نصب با اسکریپت بالا، <code>v2raydar</code> را اجرا کنید (در ویندوز <code>v2raydar.exe</code>). در اولین اجرا فایل <code>configs.yaml</code> &#x200F;با منابع اشتراک از پیش انتخاب‌شده ایجاد می‌شود.

1. &#x200F;<strong>صبر کنید تا بارگذاری کامل شود.</strong> &#x200F;برنامه منابع اشتراک شما را به صورت همزمان دریافت کرده، هر کانفیگ را از طریق شبکه واقعی بررسی و کانفیگ‌های کارآمد را رتبه‌بندی می‌کند. اندپوینت از همان ابتدا فعال است — کلاینت شما بلافاصله می‌تواند به آن متصل شود.
2. &#x200F;<strong>کلاینت خود را</strong> به URL اشتراک متصل کنید:

<div dir="ltr">

| &#x200F;کلاینت | &#x200F;اندپوینت |
| --- | --- |
| v2rayN / v2rayNG | `http://127.0.0.1:27141/subscription` (base64) |
| sing-box | `http://127.0.0.1:27141/subscription.txt` (متن ساده) |
| Clash Verge / Mihomo | `http://127.0.0.1:27141/mihomo.yaml` |

</div>

3. &#x200F;<strong>کلیدهای کنترل TUI:</strong>

<div dir="ltr">

| &#x200F;کلید | &#x200F;کارکرد |
| --- | --- |
| <code>↑</code> / <code>↓</code> یا <code>j</code> / <code>k</code> | &#x200F;پیمایش |
| <code>Enter</code> | &#x200F;انتخاب / تغییر وضعیت / تأیید |
| <code>Esc</code> / <code>Ctrl+H</code> | &#x200F;بازگشت |
| <code>Space</code> | &#x200F;فعال/غیرفعال کردن اشتراک |
| <code>e</code> | &#x200F;ویرایش اشتراک انتخاب‌شده |
| <code>q</code> | &#x200F;خروج |
| <code>:</code> | &#x200F;ورود به حالت فرمان |

</div>

&#x200F;فرمان‌های حالت <code>:</code>: <code>:q</code> خروج، <code>:w</code> ذخیره، <code>:a</code> افزودن، <code>:d</code> حذف، <code>:n</code> تغییر نام، <code>:u</code> تغییر URL، <code>:p</code> تغییر اولویت.

4. &#x200F;<strong>تغییر تنظیمات</strong> &#x200F;از منوی اصلی TUI (<code>Configurations</code>) یا ویرایش مستقیم <code>configs.yaml</code> — تغییرات در بروزرسانی بعدی اعمال می‌شوند. تنظیمات کلیدی: <code>top_n</code>، <code>refresh_seconds</code>، <code>sharing.enabled</code>، <code>probe.mode</code>.
5. &#x200F;<strong>خروج</strong> &#x200F;با <code>q</code> یا <code>:q</code>. با خروج، اندپوینت متوقف می‌شود.

### &#x200F;حالت‌های اجرا

<div dir="ltr">

```bash
v2raydar                # TUI + اندپوینت اشتراک محلی
v2raydar --no-tui       # بدون رابط گرافیکی — فقط اندپوینت و لاگ‌ها
v2raydar --once         # یک بار بروزرسانی، چاپ نتایج، خروج
v2raydar --portable     # نگهداری داده‌ها در کنار فایل اجرایی
v2raydar --uninstall    # حذف داده‌های برنامه و قوانین فایروال
```

</div>

&#x200F;کاربران ویندوز <code>v2raydar</code> را با <code>v2raydar.exe</code> &#x200F;جایگزین کنند. در macOS فایل <code>.app</code> &#x200F;بسته‌بندی‌شده را یکبار باز کنید تا Gatekeeper آن را به خاطر بسپارد.

## &#x200F;⚙️ مرور تنظیمات پیش‌فرض

<details>
  <summary>&#x200F;👣 <strong>configs.yaml</strong> — جدول تمام کلیدها، مقادیر پیش‌فرض و عملکرد. توضیحات کامل در <a href="guide.md">راهنمای توسعه‌دهندگان</a>.</summary>

<div dir="ltr">

| &#x200F;کلید | &#x200F;پیش‌فرض | &#x200F;توضیح |
| --- | --- | --- |
| <code>bind</code> | <code>127.0.0.1:27141</code> | &#x200F;آدرس محلی HTTP |
| <code>top_n</code> | <code>10</code> | &#x200F;تعداد کانفیگ‌های کارآمد منتشر شده |
| <code>refresh_seconds</code> | <code>300</code> | &#x200F;فاصله بروزرسانی خودکار (ثانیه) |
| <code>encoded_subscription</code> | <code>true</code> | &#x200F;برگرداندن base64 برای <code>/subscription</code> |
| <code>prioritize_stability</code> | <code>true</code> | &#x200F;اولویت با کانفیگ‌های پایدار قبلی |
| <code>return_configs_asap</code> | <code>false</code> | &#x200F;انتشار سریع کانفیگ‌های کارآمد |
| <code>scan_all_configs</code> | <code>false</code> | &#x200F;بررسی تمام کانفیگ‌ها |
| <code>fetch_timeout_ms</code> | <code>30000</code> | &#x200F;زمان انتظار دریافت هر منبع |
| <code>fetch_concurrency</code> | <code>8</code> | &#x200F;تعداد منابع همزمان |
| <code>max_subscription_bytes</code> | <code>33554432</code> | &#x200F;حداکثر اندازه هر منبع |
| <code>use_cache_only</code> | <code>false</code> | &#x200F;فقط استفاده از پایگاه داده |
| <code>emergency_config</code> | <code>null</code> | &#x200F;پل شبکه اضطراری |
| <code>clean_offlines_after_days</code> | <code>7</code> | &#x200F;روزهای نگهداری کانفیگ غیرفعال |
| <code>sharing.enabled</code> | <code>false</code> | &#x200F;اشتراک‌گذاری LAN |
| <code>sharing.require_token</code> | <code>false</code> | &#x200F;نیاز به توکن برای LAN |
| <code>sharing.token</code> | <code>null</code> | &#x200F;توکن اشتراک‌گذاری |
| <code>proxy.enabled</code> | <code>false</code> | &#x200F;شروع پروکسی SOCKS5/HTTP پایدار |
| <code>proxy.port</code> | <code>27910</code> | &#x200F;پورت پروکسی مختلط SOCKS5/HTTP |
| <code>proxy.discoverable</code> | <code>false</code> | &#x200F;اتصال به 0.0.0.0 و قانون فایروال برای LAN |
| <code>proxy.health_check_url</code> | <code>https://www.gstatic.com/generate_204</code> | &#x200F;URL تست سلامت پروکسی |
| <code>proxy.health_check_interval_seconds</code> | <code>60</code> | &#x200F; ثانیه بین بررسی‌های سلامت |
| <code>probe.mode</code> | <code>active</code> | &#x200F;حالت بررسی |
| <code>probe.sing_box_path</code> | <code>null</code> | &#x200F;مسیر sing-box |
| <code>probe.connect_timeout_ms</code> | <code>5000</code> | &#x200F;زمان اتصال TCP |
| <code>probe.active_timeout_ms</code> | <code>30000</code> | &#x200F;زمان تست HTTP |
| <code>probe.startup_timeout_ms</code> | <code>5000</code> | &#x200F;زمان راه‌اندازی پروکسی |
| <code>probe.concurrency</code> | <code>16</code> | &#x200F;تعداد بررسی هم‌زمان |
| <code>probe.batch_size</code> | <code>20</code> | &#x200F;اندازه اولیه دسته |
| <code>probe.process_concurrency</code> | <code>null</code> | &#x200F;تعداد فرآیند همزمان |
| <code>probe.test_url</code> | <code>https://www.gstatic.com/generate_204</code> | &#x200F;URL تست |
| <code>probe.accepted_statuses</code> | <code>[204, 200]</code> | &#x200F;کدهای وضعیت موفق |
| <code>probe.download_url</code> | <code>null</code> | &#x200F;URL تست پهنای باند |
| <code>probe.download_bytes_limit</code> | <code>1048576</code> | &#x200F;حداکثر بایت تست سرعت |
| <code>geoip_db_path</code> | <code>null</code> | &#x200F;مسیر اختیاری پایگاه GeoIP |
| <code>subscriptions</code> | <em>منابع پیش‌انتخاب</em> | &#x200F;فهرست منابع اشتراک |

</div>

</details>

## &#x200F;🌐 نکاتی برای شبکه‌های محدود

- &#x200F;در شبکه‌های بسیار محدود، کانفیگ‌های قبلاً بررسی‌شده در پایگاه داده ذخیره شده و از طریق <code>use_cache_only: true</code> &#x200F;قابل بازیابی هستند.
- &#x200F;به صورت پیش‌فرض، اگر برخی URLهای اشتراک HTTP متصل نشوند اما یک کانفیگ کارآمد وجود داشته باشد، برنامه از آن کانفیگ به عنوان پل برای تلاش مجدد استفاده می‌کند. اگر هیچ کانفیگ کارآمدی ندارید اما خودتان یک کانفیگ کارآمد دارید، آن را در <code>emergency_config</code> &#x200F;فایل <code>configs.yaml</code> &#x200F;وارد کنید.

## &#x200F;📡 اتصال کلاینت‌های رایج به V2RayDAR

- &#x200F;<strong>v2rayN (همین رایانه)</strong> — مقدار <code>bind: 127.0.0.1:27141</code> &#x200F;را حفظ کرده و <code>http://127.0.0.1:27141/subscription</code> &#x200F;را به عنوان URL اشتراک اضافه کنید.
- &#x200F;<strong>v2rayNG / گوشی در همان Wi-Fi</strong> — به IP LAN رایانه (مثلاً <code>192.168.1.23:27141</code>) متصل شوید، <code>sharing.enabled</code> &#x200F;را فعال کرده و سپس در گوشی از <code>http://192.168.1.23:27141/subscription</code> &#x200F;استفاده کنید. ابتدا از گوشی <code>/health</code> &#x200F;را بررسی کنید.

&#x200F;راهنمای کامل نصب کلاینت، اشتراک‌گذاری با محافظت توکن و جزئیات فایروال هر سیستم‌عامل در <a href="guide.md">راهنمای توسعه‌دهندگان</a> موجود است.

### &#x200F;📱 پروکسی پایدار برای ترافیک برنامه‌ها

V2RayDAR می‌تواند یک پروکسی SOCKS5/HTTP پایدار در کنار endpoint اشتراک اجرا کند. هر برنامه‌ای روی سیستم — تلگرام، مرورگرها، curl، Python — می‌تواند ترافیک را از طریق آن مسیریابی کند.

**فعال‌سازی در `configs.yaml`:**
```yaml
proxy:
  enabled: true
  port: 27910
  discoverable: false   # true = دسترسی LAN + قانون فایروال
```

**استفاده محلی (روی دستگاه اجراکنندهی V2RayDAR):**
```bash
curl --socks5 127.0.0.1:27910 https://api.ipify.org
```

**استفاده LAN (گوشی در همان Wi-Fi):**
1. `proxy.discoverable: true` را تنظیم کنید — V2RayDAR قانون فایروال اضافه کرده و به `0.0.0.0` متصل می‌شود.
2. IP LAN دستگاه اجراکنندهی V2RayDAR را از پنل TUI در بخش **Network** پیدا کنید (یا `ipconfig` / `ip addr` اجرا کنید). به عنوان مثال `192.168.1.2`.
3. **تلگرام:** `YOUR_LAN_IP` را با IP LAN واقعی خود جایگزین کنید و این URL را روی گوشی باز کنید:

   ```
   https://t.me/socks?server=YOUR_LAN_IP&port=27910
   ```

   به عنوان مثال، اگر IP LAN شما `192.168.1.2` باشد:
   ```
   https://t.me/socks?server=192.168.1.2&port=27910
   ```

   یا دستی: تلگرام → تنظیمات → داده و ذخیره‌سازی → تنظیمات پروکسی → افزودن پروکسی:
   - نوع: **SOCKS5** یا **HTTP**
   - میزبان: `YOUR_LAN_IP` (آیپی نشان داده شده در پنل TUI)
   - پورت: `27910`

پروکسی به صورت خودکار به بهترین پیکربندی بعدی سوئیچ می‌کند.

## &#x200F;🤝 مشارکت

&#x200F;پذیرای مشارکت شما هستیم! برای گزارش باگ، درخواست امکان جدید، پرسش یا پیشنهاد، یک Issue باز کنید یا Pull Request ارسال نمایید.

## &#x200F;🗺 نقشه راه

- [ ] &#x200F;افزودن برنامه GUI بین‌پلتفرمی در کنار TUI با Tauri.
- [ ] &#x200F;استخراج کانفیگ‌های V2Ray از متن هر وب‌سایت — ترجیحاً سایت‌های سبک‌تر از نظر جاوااسکریپت، و FireCrawl یا Obscura به عنوان جایگزین برای سایت‌های سنگین جاوااسکریپت.
- [ ] &#x200F;اندپوینت‌های خصوصی با رمز عبور و احراز هویت: کاربران بتوانند اندپوینت خصوصی خود را از طریق یک اندپوینت ملی در دسترس، دریافت کنند.

## &#x200F;👨‍💻 سلب مسئولیت

&#x200F;این برنامه &#x200E;"همان‌طور که هست" &#x200F;ارائه شده و هیچ‌گونه ضمانتی ندارد.

&#x200F;توسعه‌دهنده کانفیگ‌های سازگار با V2Ray ایجاد یا توزیع نمی‌کند و در قبال اشتراک‌های V2Ray که کاربر اسکن و به آن‌ها متصل می‌شود مسئولیتی ندارد.

## &#x200F;☕️ تماس و حمایت مالی

### &#x200F;💬 تماس

<p align="center">
<a href="https://t.me/TechKrakenBot">
  <img src="https://img.shields.io/badge/Telegram-2CA5E0?style=for-the-badge&logo=telegram&logoColor=white" alt="Telegram Bot">
</a>
</p>

### &#x200F;💎 حمایت مالی از طریق TON

&#x200F;اگر این پروژه برای شما مفید بوده، می‌توانید از طریق بلاکچین TON از توسعه آن حمایت کنید:

<div dir="ltr">

```
ton://transfer/TechKraken.ton
```

```
UQCGk4IU5nm6dYWjXTx6vSQVOtKO4LQg3m8cRcq1eQo7vhCl
```

</div>

</div>
