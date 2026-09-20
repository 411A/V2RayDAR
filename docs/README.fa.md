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
  <em>تشخیص و بازشناسی V2Ray &#x200F;— مانند <code>v2ray</code> + <code>radar</code> تلفظ می‌شود.</em><br>
  <a href="https://github.com/411A/V2RayDAR/releases/latest"><img src="https://img.shields.io/github/v/release/411A/V2RayDAR" alt="آخرین نسخه"></a>
  <a href="../LICENSE"><img src="https://img.shields.io/github/license/411A/V2RayDAR" alt="مجوز: AGPL-3.0"></a>
  <a href="https://github.com/411A/V2RayDAR/actions/workflows/rust.yml"><img src="https://github.com/411A/V2RayDAR/actions/workflows/rust.yml/badge.svg" alt="وضعیت Rust CI"></a>
</p>

<p align="center">
  <strong>آن را یک‌بار روی هر دستگاه همیشه‌روشن اجرا کنید — گوشی قدیمی، کامپیوتر، رزبری‌پای یا سرور خانگی — و V2RayDAR به‌طور مداوم بهترین کانفیگ‌های سالم را پیدا، بررسی و به همه دستگاه‌های شبکه محلی شما ارائه می‌کند. همچنین یک پروکسی استاندارد SOCKS5/HTTP ارائه می‌کند، پس همه دستگاه‌های شبکه محلی شما به یک اتصال V2Ray سالم دسترسی دارند — بدون نیاز به کلاینت V2Ray.</strong>
</p>

<p align="center">
  سرویس سریع Rust با داشبورد وب داخلی که منابع اشتراک &#x200F;V2Ray / Clash / Mihomo &#x200F;را دریافت می‌کند، آن‌ها را از طریق شبکه واقعی شما با <code>sing-box</code> اعتبارسنجی می‌کند، کانفیگ‌های واقعاً کارآمد را رتبه‌بندی کرده و بهترین‌ها را در یک URL اشتراک محلی بازنشر می‌کند تا کلاینت &#x200F;v2rayN / v2rayNG / sing-box / Clash Verge / Mihomo &#x200F;شما به آن وصل شود. رابط ترمینالی اختیاری (<code>--tui</code>) چند اقدام نگهداری اضافه را پوشش می‌دهد.
</p>

<p align="center">
  &#x200F;📘 <a href="guide.md">راهنمای جامع</a>
  • 🧠 <a href="https://deepwiki.com/411A/V2RayDAR">Ask DeepWiki</a>
  • &#x200F;📡 <a href="#x200f-اندپوینتهای-اشتراک">اندپوینت‌ها</a>
  • &#x200F;🌐 <a href="#x200f-پروکسی-پایدار">پروکسی</a>
</p>

## &#x200F;📖 فهرست

- &#x200F;[✨ قابلیت‌ها](#x200f-قابلیتها)
- &#x200F;[🖥️ رابط‌ها](#x200f-رابطها)
- &#x200F;[📦 نصب](#x200f-نصب)
  - &#x200F;[<img src="https://cdn.svglogos.dev/logos/linux-tux.svg" alt="" width="16" height="16"> Linux / macOS](#linux--macos)
  - &#x200F;[<img src="https://cdn.svglogos.dev/logos/microsoft-windows-icon.svg" alt="" width="16" height="16"> Windows](#windows-powershell)
  - &#x200F;[<img src="https://cdn.svglogos.dev/logos/android-icon.svg" alt="" width="16" height="16"> Android / Termux](#android--termux)
- &#x200F;[🔰 شروع سریع](#x200f-شروع-سریع)
- &#x200F;[⚙️ تنظیمات](#x200f-تنظیمات)
- &#x200F;[📡 اتصال کلاینت‌ها](#x200f-اتصال-کلاینثا)
- &#x200F;[🌐 پروکسی پایدار](#x200f-پروکسی-پایدار)
- &#x200F;[🔒 شبکه‌های محدود](#x200f-شبکههای-محدود)
- &#x200F;[🤝 مشارکت](#x200f-مشارکت)
- &#x200F;[⚠️ سلب مسئولیت و امنیت](#x200f-سلب-مسئولیت-و-امنیت)
- &#x200F;[☕ حمایت](#x200f-حمایت)
- &#x200F;[📄 مجوز](#x200f-مجوز)

## &#x200F;✨ قابلیت‌ها

### &#x200F;🔎 کشف و بررسی

- &#x200F;**دریافت همزمان** — دریافت همزمان اشتراک‌ها از تعداد نامحدودی منابع.
- &#x200F;**پشتیبانی چندفرمتی** — فرمت‌های خام، <code>base64</code>، <code>JSON</code> و <code>YAML</code> &#x200F;و لینک‌های اشتراک <code>vmess</code>، <code>vless</code>، <code>trojan</code>، <code>ss</code>، <code>ssr</code>، <code>hysteria2</code>، <code>hy2</code>، <code>tuic</code>.
- &#x200F;**بررسی واقعی** — بررسی هر کانفیگ از طریق شبکه واقعی شما با <code>sing-box</code> &#x200F;(یعنی واقعاً یک URL تست را از مسیر پروکسی بارگذاری می‌کند).
- &#x200F;**رتبه‌بندی هوشمند** — کانفیگ‌های واقعاً کارآمد رتبه‌بندی می‌شوند و سالم‌ترین‌ها در صدر قرار می‌گیرند.

### &#x200F;🔄 فرمت‌ها و خروجی

- &#x200F;**ورودی Clash/Mihomo** &#x200F;— کافیست یک URL اشتراک Mihomo اضافه کنید تا V2RayDAR تمام ورودی‌های پروکسی را به صورت خودکار استخراج کند.
- &#x200F;**تبدیل دوطرفه** &#x200F;— تبدیل بین لینک‌های اشتراک V2Ray و ورودی‌های پروکسی Clash/Mihomo YAML.
- &#x200F;**خروجی دو فرمت** &#x200F;— کانفیگ‌های کارآمد هم به صورت لینک اشتراک <code>/subscription</code> &#x200F;و هم به صورت کانفیگ کامل Mihomo YAML <code>/mihomo.yaml</code> &#x200F;ارائه می‌شوند.
- &#x200F;**یک اشتراک همیشه به‌روز** — انتشار مجدد بهترین کانفیگ‌های کارآمد در یک URL محلی، طوری که هر کلاینت سازگار یک اشتراک همیشه به‌روز داشته باشد.

### &#x200F;🌐 پروکسی پایدار

- &#x200F;**پروکسی همیشه‌روشن** — یک فرآیند <code>sing-box</code> را با بهترین کانفیگ روشن نگه می‌دارد و یک پورت پروکسی محلی در اختیار هر برنامه‌ای می‌گذارد — بدون نیاز به کلاینت V2Ray.
- &#x200F;**سوئیچ خودکار** — هنگام از کار افتادن کانفیگ فعلی، خودکار به بهترین کانفیگ بعدی سوئیچ می‌کند و در هر چرخه بروزرسانی هم به کانفیگ بهتر مهاجرت می‌کند.

### &#x200F;📱 اشتراک‌گذاری LAN و QR کد

- &#x200F;**اشتراک‌گذاری LAN** — اشتراک‌گذاری اختیاری در LAN با محافظت اختیاری توکن، مناسب برای استفاده گوشی از همان منبع اشتراک.
- &#x200F;**پروکسی در LAN** — با <code>proxy.discoverable: true</code> به <code>0.0.0.0</code> بایند می‌شود و قوانین فایروال اضافه می‌کند. راه‌اندازی تک‌ضربه‌ای تلگرام: <code>https://t.me/socks?server=192.0.2.2&port=27910</code>.
- &#x200F;**برگه QR کد** — برای اشتراک LAN و پروکسی تلگرام QR قابل اسکن بسازید: از زبانه Share داشبورد، دکمه‌های QR هر کانفیگ در زبانه Configs، یا گزینه <code>QR Codes: Generate & View</code> منوی اصلی TUI (دسکتاپ، ذخیره در <code>v2raydar_data/QRCodes.jpg</code>).

### &#x200F;🔒 شبکه‌های محدود

- &#x200F;**پایگاه کانفیگ‌های بررسی‌شده** — کارکرد در شبکه‌های محدود از طریق کانفیگ‌های قبلاً بررسی‌شده در پایگاه داده، پل شبکه داخلی یا <code>emergency_config</code>.

## &#x200F;🖥️ رابط‌ها

### &#x200F;🌐 داشبورد وب (پیش‌فرض)

&#x200F;داشبورد وب رابط پیش‌فرض است و همه کارهای روزمره را از یک صفحه مرورگر پوشش می‌دهد: آمار زنده Overview، کانفیگ‌های رتبه‌بندی‌شده Configs با QR هر سطر، مدیریت Subscriptions (افزودن، ویرایش، روشن/خاموش، حذف، مرتب‌سازی با درگ‌انددراپ)، همه تنظیمات در زبانه Settings، زبانه Proxy، اشتراک‌گذاری در LAN، لاگ‌های زنده Logs و برگه QR برای راه‌اندازی گوشی‌ها.

&#x200F;پس از اجرای برنامه، نشانی http://127.0.0.1:27141 را در مرورگر باز کنید.

<p align="center">
  <img src="../assets/WebUI_v0.9.0.png" alt="داشبورد وب V2RayDAR" width="100%">
</p>

<p align="center">
  <img src="../assets/WebUI-Mobile_v0.9.0.png" alt="داشبورد وب V2RayDAR در موبایل" width="300">
</p>

### &#x200F;🖥️ رابط ترمینالی اختیاری (<code>--tui</code>)

&#x200F;با <code>v2raydar --tui</code> اجرا کنید تا رابط ترمینالی کلاسیک را در کنار داشبورد و اندپوینت داشته باشید. این حالت اضافه‌تر پاک‌سازی کش و بازگشت به تنظیمات پیش‌فرض را هم ارائه می‌کند — بقیه قابلیت‌ها در داشبورد هم هستند.

<p align="center">
  <img src="../assets/Windows_TUI_v0.9.0.png" alt="رابط ترمینالی V2RayDAR در ویندوز" width="100%">
</p>

## &#x200F;📦 نصب

&#x200F;نصب‌کننده مخصوص سیستم‌عامل خود را اجرا کنید. با تنظیمات پیش‌فرض کار می‌کند (Enter برای پذیرش) و در صورت رد، مرحله‌به‌مرحله پیش می‌رود؛ به‌روزرسانی درجا داده‌های شما را حفظ می‌کند.

&#x200F;کار نصب‌کننده: تشخیص پلتفرم، دانلود آخرین نسخه همراه <code>sing-box</code>، بررسی هش SHA-256، شناسایی نصب‌های موجود و پیشنهاد به‌روزرسانی (با حفظ <code>data.db</code> و <code>v2raydar_data/</code>) و به صورت پیش‌فرض بدون نیاز به sudo.

#### Linux / macOS <img src="https://cdn.svglogos.dev/logos/linux-tux.svg" alt="Linux" width="20" height="20" align="texttop">

<div dir="ltr" align="left">

```bash
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh
```

</div>

#### Windows (PowerShell) <img src="https://cdn.svglogos.dev/logos/microsoft-windows-icon.svg" alt="Windows" width="20" height="20" align="texttop">

<div dir="ltr" align="left">

```powershell
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
```

</div>

#### Android / Termux <img src="https://cdn.svglogos.dev/logos/android-icon.svg" alt="Android" width="20" height="20" align="texttop">

<div dir="ltr" align="left">

```bash
pkg update -y && apt update && apt full-upgrade -y && pkg install -y curl tar && curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | bash && cd ~/V2RayDAR && ./v2raydar
```

</div>

### &#x200F;👤 نصب کاربری

&#x200F;فایل باینری در <code>~/.local/bin</code>، داده‌ها در پوشه خانه:

<div dir="ltr" align="left">

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --user

# Windows
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
# Then choose option 2 when prompted
```

</div>

* **پایان اتصال:** `Ctrl + C`
* **شروع:** `cd ~/V2RayDAR && ./v2raydar`

### &#x200F;📥 دانلود دستی

&#x200F;آرشیو مخصوص سیستم‌عامل خود را از <a href="https://github.com/411A/V2RayDAR/releases/latest">Releases</a> دانلود کرده و اجرا کنید.

&#x200F;<strong>حالت پرتابل</strong> (توصیه‌شده) — همه فایل‌ها در یک پوشه. پوشه خودکفا (<code>sing-box</code> همراه یا <code>v2raydar_data/</code> موجود در کنار فایل اجرایی) به‌طور خودکار تشخیص داده می‌شود؛ <code>--portable</code> آن را در هر مکانی اجباری می‌کند. حالت پرتابل در <code>Desktop/V2RayDAR</code> &#x200F;(یا در صورت نبود پوشه رومیزی، در <code>~/V2RayDAR</code>) نصب می‌شود.

## &#x200F;🔰 شروع سریع

&#x200F;پس از نصب با اسکریپت بالا، <code>v2raydar</code> را اجرا کنید (در ویندوز <code>v2raydar.exe</code>). در اولین اجرا تنظیمات پیش‌فرض و منابع اشتراک از پیش انتخاب‌شده در <code>data.db</code> ساخته می‌شوند. هنگام به‌روزرسانی، فایل <code>configs.yaml</code> موجود خودکار به <code>data.db</code> منتقل می‌شود.

1. &#x200F;<strong>صبر کنید تا بارگذاری کامل شود.</strong> &#x200F;برنامه منابع اشتراک شما را به صورت همزمان دریافت کرده، هر کانفیگ را از طریق شبکه واقعی بررسی و کانفیگ‌های کارآمد را رتبه‌بندی می‌کند. اندپوینت از همان ابتدا فعال است — کلاینت شما بلافاصله می‌تواند به آن متصل شود.
2. &#x200F;<strong>کلاینت خود را</strong> به یکی از اندپوینت‌های اشتراک زیر متصل کنید:
3. &#x200F;<strong>از داشبورد استفاده کنید</strong> در <code>http://127.0.0.1:27141</code> — زبانه‌های Overview، Configs، Subscriptions، Settings، Proxy، Logs و Share. همه چیز بی‌درنگ در پایگاه داده و ران‌تایم زنده ذخیره می‌شود؛ تنظیماتی که روی رفرش اثر می‌گذارند در چرخه بعدی اعمال می‌شوند (داشبورد هنگام ذخیره اعلام می‌کند)، ولی تغییر فهرست منابع بلافاصله واکشی تازه انجام می‌دهد.
4. &#x200F;<strong>تغییر تنظیمات</strong> &#x200F;از زبانه Settings داشبورد (یا صفحه Configurations رابط ترمینالی با <code>--tui</code>) — تغییرها بی‌درنگ ذخیره می‌شوند و تنظیماتی که روی رفرش اثر می‌گذارند در چرخه زمان‌بندی‌شده بعدی یا با رفرش دستی اعمال می‌شوند. تنظیمات کلیدی: <code>top_n</code>، <code>refresh_seconds</code>، <code>ping_seconds</code>، <code>sharing.enabled</code>، <code>probe.mode</code>. تنظیمات جدید نسخه‌های تازه‌تر خودکار مقدار پیش‌فرض می‌گیرند؛ مقادیر ذخیره‌شده شما هرگز بازنویسی نمی‌شوند.
5. &#x200F;<strong>خروج</strong> &#x200F;با <code>Ctrl + C</code>. با خروج برنامه، اندپوینت متوقف می‌شود.

### &#x200F;📡 اندپوینت‌های اشتراک

&#x200F;سه فرمت خروجی، یکی برای هر خانواده کلاینت:

<div dir="ltr">

| &#x200F;کلاینت | &#x200F;اندپوینت |
| --- | --- |
| v2rayN / v2rayNG | `http://127.0.0.1:27141/subscription` (base64) |
| sing-box | `http://127.0.0.1:27141/subscription.txt` (متن ساده) |
| Clash Verge / Mihomo | `http://127.0.0.1:27141/mihomo.yaml` |

</div>

### &#x200F;⚙️ حالت‌های اجرا

<div dir="ltr" align="left">

```bash
v2raydar                # حالت ساکت — فقط راهنمای مرورگر، بدون لاگ
v2raydar --no-tui       # بدون TUI — جزئیات و لاگ‌ها
v2raydar --tui          # TUI + اندپوینت اشتراک محلی
v2raydar --once         # یک بار بروزرسانی، چاپ نتایج، خروج
v2raydar --portable     # نگهداری داده‌ها در کنار فایل اجرایی (تشخیص خودکار در پوشه پرتابل)
v2raydar --uninstall    # حذف داده‌های برنامه و قوانین فایروال
```

</div>

&#x200F;کاربران ویندوز <code>v2raydar</code> را با <code>v2raydar.exe</code> &#x200F;جایگزین کنند. در macOS فایل <code>.app</code> &#x200F;بسته‌بندی‌شده را یکبار باز کنید تا Gatekeeper آن را به خاطر بسپارد.

<details>
  <summary>&#x200F;🖥️ <strong>کنترل‌های صفحه‌کلید TUI</strong></summary>

<div dir="ltr">

| &#x200F;کلید | &#x200F;کارکرد |
| --- | --- |
| <code>↑</code> / <code>↓</code> یا <code>j</code> / <code>k</code> | &#x200F;پیمایش |
| <code>Enter</code> | &#x200F;انتخاب / تغییر وضعیت / تأیید |
| <code>Esc</code> / <code>Ctrl+H</code> | &#x200F;بازگشت |
| <code>Space</code> | &#x200F;فعال/غیرفعال کردن اشتراک |
| <code>e</code> | &#x200F;ویرایش اشتراک انتخاب‌شده |
| <code>Ctrl+R</code> | &#x200F;بروزرسانی دستی (دریافت دوباره، فقط وقتی اجرا نمی‌شود) |
| <code>Ctrl+P</code> | &#x200F;پینگ دستی پیکربندی‌های ذخیره‌شده (وقتی چرخه‌ای در حال اجراست نه) |
| <code>q</code> | &#x200F;خروج |
| <code>:</code> | &#x200F;ورود به حالت فرمان |

</div>

&#x200F;فرمان‌های حالت <code>:</code>: <code>:q</code> خروج، <code>:w</code> ذخیره، <code>:a</code> افزودن، <code>:d</code> حذف، <code>:n</code> تغییر نام، <code>:u</code> تغییر URL، <code>:p</code> تغییر اولویت، <code>:r</code> بروزرسانی، <code>:ping</code> پینگ.

</details>

## &#x200F;⚙️ تنظیمات

<details>
  <summary>&#x200F;👣 <strong>مشاهده همه تنظیمات و پیش‌فرض‌ها</strong></summary>

<div dir="ltr">

| &#x200F;کلید | &#x200F;پیش‌فرض | &#x200F;توضیح |
| --- | --- | --- |
| <code>bind</code> | <code>127.0.0.1:27141</code> | &#x200F;آدرس محلی HTTP |
| <code>top_n</code> | <code>10</code> | &#x200F;تعداد کانفیگ‌های کارآمد منتشر شده |
| <code>refresh_seconds</code> | <code>900</code> | &#x200F;فاصله بروزرسانی خودکار (ثانیه) |
| <code>ping_seconds</code> | <code>300</code> | &#x200F;فاصله پینگ دوباره پیکربندی‌های ذخیره‌شده بدون دریافت دوباره (ثانیه)؛ 0 غیرفعال می‌کند. اگر تعداد تأییدشده کمتر از <code>top_n</code> باشد، پینگ پیکربندی‌های دیده‌شده قبلی در پایگاه داده را نیز برای تکمیل بررسی می‌کند |
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
| <code>proxy.rotating_proxy</code> | <code>true</code> | &#x200F;با true پروکسی هر چرخه به کانفیگ کم‌پینگ‌تر می‌رود؛ با false کانفیگ فعلی تا وقتی آنلاین است می‌ماند |
| <code>proxy.health_check_url</code> | <code>https://cp.cloudflare.com</code> | &#x200F;URL تست سلامت پروکسی |
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
| <code>geoip_db_path</code> | <code>null</code> | &#x200F;مسیر اختیاری فایل <code>GeoLite2-Country.mmdb</code> یا پوشه zoneهای کشورها (<code>zones.txt</code> یا فایل‌های قدیمی <code>&lt;cc&gt;.zone</code>). اگر <code>null</code> باشد از <code>&lt;data-root&gt;/geoip</code> استفاده می‌شود (اول پایگاه MaxMind بعد zoneها؛ هر دو توسط نصب‌کننده به‌روز می‌شوند). Country data: GeoLite2 by MaxMind (CC BY-SA 4.0); fallback zones by ipdeny |
| <code>subscriptions</code> | <em>منابع پیش‌انتخاب</em> | &#x200F;فهرست منابع اشتراک |

</div>

&#x200F;برای رفتار دقیق، مثال‌ها، نکات مهاجرت و تنظیمات پیشرفته، به <a href="guide.md">راهنمای جامع</a> مراجعه کنید.

</details>

### &#x200F;🗄️ مشاهده پایگاه داده

می‌توانید فایل پایگاه داده (`data.db`) را با دانلود این نرم‌افزار رایگان مشاهده یا تغییر دهید (تغییر توصیه نمی‌شود):

https://sqlitebrowser.org/dl

&#x200F;توجه داشته باشید که تغییر مستقیم پایگاه داده ممکن است باعث اختلال در سیستم شود.

## &#x200F;📡 اتصال کلاینت‌ها

### &#x200F;v2rayN (همین رایانه)

&#x200F;مقدار <code>bind: 127.0.0.1:27141</code> &#x200F;را حفظ کرده و <code>http://127.0.0.1:27141/subscription</code> &#x200F;را به عنوان URL اشتراک اضافه کنید.

### &#x200F;v2rayNG / گوشی در همان Wi-Fi

&#x200F;به IP LAN رایانه (مثلاً <code>192.0.2.23:27141</code>) متصل شوید، <code>sharing.enabled</code> &#x200F;را فعال کرده و سپس در گوشی از <code>http://192.0.2.23:27141/subscription</code> &#x200F;استفاده کنید. ابتدا از گوشی <code>/health</code> &#x200F;را بررسی کنید.

### &#x200F;sing-box (فرمت متنی)

<div dir="ltr" align="left">

```text
http://127.0.0.1:27141/subscription.txt
```

</div>

### &#x200F;Clash Verge / Mihomo (فرمت YAML)

<div dir="ltr" align="left">

```text
http://127.0.0.1:27141/mihomo.yaml
```

</div>

&#x200F;این URL را مستقیم در تنظیمات پروفایل/اشتراک کلاینت Clash خود وارد کنید. V2RayDAR یک کانفیگ کامل Mihomo با ورودی‌های پروکسی، گروه <code>url-test</code> و قانون فراگیر <code>MATCH</code> تولید می‌کند.

&#x200F;راهنمای کامل نصب کلاینت، اشتراک‌گذاری با محافظت توکن و جزئیات فایروال هر سیستم‌عامل در <a href="guide.md">راهنمای توسعه‌دهندگان</a> موجود است.

## &#x200F;🌐 پروکسی پایدار

V2RayDAR می‌تواند یک پروکسی SOCKS5/HTTP پایدار در کنار endpoint اشتراک اجرا کند. هر برنامه‌ای روی سیستم — تلگرام، مرورگرها، curl، Python — می‌تواند ترافیک را از طریق آن مسیریابی کند.

**فعال‌سازی از زبانه Proxy داشبورد یا سطر Proxy رابط ترمینالی**
(<code>enabled: true</code>، پورت <code>27910</code>، مقدار <code>discoverable: true</code> یعنی دسترسی LAN + قانون فایروال).

### &#x200F;استفاده محلی (روی دستگاه اجراکننده V2RayDAR)

<div dir="ltr" align="left">

```bash
# SOCKS5
curl --socks5 127.0.0.1:27910 https://api.ipify.org

# HTTP
curl --proxy http://127.0.0.1:27910 https://api.ipify.org
```

</div>

### &#x200F;استفاده LAN (گوشی در همان Wi-Fi)

1. `proxy.discoverable: true` را تنظیم کنید — V2RayDAR قانون فایروال اضافه کرده و به `0.0.0.0` متصل می‌شود.
2. IP LAN دستگاه اجراکننده V2RayDAR را در زبانه Overview داشبورد زیر **Network** پیدا کنید (یا پنل **Current Configuration** رابط ترمینالی، یا `ipconfig` / `ip addr` اجرا کنید). به عنوان مثال `192.0.2.2`.

### &#x200F;تلگرام

`YOUR_LAN_IP` را با IP LAN واقعی خود جایگزین کنید و این URL را روی گوشی باز کنید:

<div dir="ltr" align="left">

```
https://t.me/socks?server=YOUR_LAN_IP&port=27910
```

</div>

به عنوان مثال، اگر IP LAN شما `192.0.2.2` باشد:

<div dir="ltr" align="left">

```
https://t.me/socks?server=192.0.2.2&port=27910
```

</div>

یا دستی: تلگرام → تنظیمات → داده و ذخیره‌سازی → تنظیمات پروکسی → افزودن پروکسی:
- نوع: **SOCKS5** یا **HTTP**
 - میزبان: `YOUR_LAN_IP` (آیپی نشان داده‌شده در داشبورد یا پنل TUI)
 - پورت: `27910`

### &#x200F;پروکسی سیستمی اندروید

تنظیمات → WiFi → نگه داشتن روی شبکه → Modify → پیشرفته → پروکسی → دستی → سرور: `YOUR_LAN_IP`، پورت: `27910`.

## &#x200F;🔒 شبکه‌های محدود

&#x200F;سه مکانیزم پشتیبان، به ترتیب:

1. &#x200F;**کانفیگ‌های قبلاً بررسی‌شده** — در پایگاه داده ذخیره شده و از طریق <code>use_cache_only: true</code> &#x200F;قابل بازیابی هستند.
2. &#x200F;**پل داخل شبکه** — اگر برخی URLهای اشتراک HTTP متصل نشوند اما یک کانفیگ کارآمد وجود داشته باشد، برنامه از آن برای تلاش مجدد استفاده می‌کند. به صورت پیش‌فرض خودکار انجام می‌شود؛ اگر کانفیگ کارآمدی ندارید اما خودتان یکی دارید، آن را به‌عنوان <code>emergency_config</code> وارد کنید (زبانه Settings داشبورد یا صفحه Configurations رابط ترمینالی).
3. &#x200F;**کانفیگ اضطراری** — لینک سالم شناخته‌شده خودتان به عنوان پشتیبان صریح.

## &#x200F;🤝 مشارکت

&#x200F;پذیرای مشارکت شما هستیم! برای گزارش باگ، درخواست امکان جدید، پرسش یا پیشنهاد، یک Issue باز کنید یا Pull Request ارسال نمایید. هر بازخوردی بسیار ارزشمند است.

&#x200F;🤖 V2RayDAR را نگهدارنده انسانی آن با کمک چند دستیار هوش مصنوعی مهندسی و کدنویسی کرده است — مشارکت‌های بازبینی‌شده توسط انسان، چه از سوی افراد و چه از سوی دستیارهای هوش مصنوعی آن‌ها، به یک اندازه استقبال می‌شوند.

## &#x200F;⚠️ سلب مسئولیت و امنیت

### &#x200F;سلب مسئولیت

&#x200F;این برنامه &#x200E;"همان‌طور که هست" &#x200F;ارائه شده و هیچ‌گونه ضمانتی ندارد.

### &#x200F;کانفیگ‌های شخص ثالث

&#x200F;توسعه‌دهنده کانفیگ‌های سازگار با V2Ray ایجاد یا توزیع نمی‌کند. V2RayDAR فقط منابع اشتراکی را که خودتان پیکربندی کرده‌اید اسکن می‌کند و کانفیگ‌های سالم را روی دستگاه خودتان بازنشر می‌کند. مسئولیت URLها و کانفیگ‌هایی که اسکن، وارد و متصل می‌شوید با شماست.

### &#x200F;هشدار امنیتی

&#x200F;اپراتور سرور V2Ray که به آن متصل می‌شوید ممکن است بتواند ترافیک شما را رهگیری کرده و داده‌های رمزنگاری‌نشده را بخواند. برای استفاده روی همان دستگاه از <code>127.0.0.1:27141</code> استفاده کنید، در LANهای مشترک یا کم‌اعتماد <code>sharing.require_token: true</code> را فعال کنید و هرگز اندپوینت HTTP برنامه را در معرض اینترنت عمومی قرار ندهید. با URLهای اشتراک و لینک‌ها مثل داده حساس رفتار کنید.

## &#x200F;☕ حمایت

### &#x200F;💬 تماس

<p align="center">
<a href="https://t.me/TechKrakenBot">
  <img src="https://img.shields.io/badge/Telegram-2CA5E0?style=for-the-badge&logo=telegram&logoColor=white" alt="Telegram Bot">
</a>
</p>

### &#x200F;💎 حمایت مالی از طریق TON

&#x200F;اگر این پروژه برای شما مفید بوده، می‌توانید از طریق بلاکچین TON از توسعه آن حمایت کنید:

<div dir="ltr" align="left">

```
ton://transfer/TechKraken.ton
```

```
UQCGk4IU5nm6dYWjXTx6vSQVOtKO4LQg3m8cRcq1eQo7vhCl
```

</div>

## &#x200F;📄 مجوز

&#x200F;V2RayDAR تحت **GNU Affero General Public License نسخه 3.0 (AGPL-3.0)** منتشر می‌شود — به <a href="../LICENSE">LICENSE</a> مراجعه کنید.

</div>
