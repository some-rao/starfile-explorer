# StarFile Explorer

[![Sürüm](https://img.shields.io/github/v/release/some-rao/starfile-explorer)](../../releases/latest)
[![Derleme](https://github.com/some-rao/starfile-explorer/actions/workflows/build.yml/badge.svg)](../../actions/workflows/build.yml)
[![Lisans: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

[English](README.md)

Windows 11 Dosya Gezgini gibi görünen ve çalışan, Rust ile yazılmış bir dosya yöneticisi: çerçevesiz
akrilik pencere, başlık çubuğunda sekmeler, komut paleti, hızlı bakış, galeri ve satır içi önizlemeler.
Klasörler arka planda okunduğu için büyük klasörler pencereyi dondurmaz, arama alt klasörlerde paralel
çalışır.

![Büyük simgeler, tarihe göre gruplu](docs/large-icons.png)

## Kurulum

**Windows.** [Sürümler](../../releases) sayfasından `StarFile-Setup-<sürüm>.exe` dosyasını indirip çalıştırın.
Yalnızca sizin kullanıcınız için kurulur (`%LOCALAPPDATA%\Programs\StarFile Explorer`), yönetici izni
istemez ve Başlat menüsü kısayolunu kendisi oluşturur.

- **Güncellemeler** uygulama içinden yapılır: yeni sürüm GitHub'dan indirilir, SHA-256 ile doğrulanır ve
  pencereyi kapattığınızda devreye girer. Elle indirmeniz gerekmez. Görünüm > Güncellemeleri otomatik
  kur bunu kapatır.
- **Kaldırma** için Windows Ayarlar > Uygulamalar yolunu ya da Başlat menüsündeki girdiyi kullanın.
  Kaldırıcı programı, otomatik başlatma kaydını ve dosya yöneticisi kaydını siler; ayarlarınız
  (`%APPDATA%\StarFile`) bilerek korunur.
- Yükleyici henüz **kod imzalı değil**, bu yüzden ilk çalıştırmada "Windows kişisel bilgisayarınızı
  korudu" uyarısı çıkar: *Ek bilgi* > *Yine de çalıştır*.

`StarFile.exe` tek başına da yayınlanır; güncelleyicinin kullandığı dosya budur. Kurulum yapmadan
çalışır ama Başlat menüsü girdisi, kaldırıcı ve güncelleme devri olmaz.

**Linux.** Kullanıcınız için kurmak için:

```bash
curl -fsSL https://raw.githubusercontent.com/some-rao/starfile-explorer/main/install.sh | sh
```

`starfile` `~/.local/bin` içine kurulur ve uygulama menüsüne eklenir. Kaldırmak için aynı komutu `sh`
sonrasına `-s -- --uninstall` ekleyerek çalıştırın. `starfile-linux-x86_64.tar.gz` arşivini indirip
kurulum yapmadan da çalıştırabilirsiniz; `starfile.flatpak` dosyası
`flatpak install --user starfile.flatpak` ile kurulur.

## Neler yapar

**Arayüz**

- Explorer ile aynı düzen: başlık çubuğunda sekmeler, adres çubuğunu içeren tek araç çubuğu kartı,
  gezinti bölmesi, Giriş, Galeri ve Bu bilgisayar sayfaları.
- Pencerenin sistem başlık çubuğu yoktur: başlık, sekme şeridi ve pencere düğmeleri uygulama tarafından
  masaüstü duvar kağıdını izleyen akrilik zemin üzerine çizilir. Büyütüldüğünde pencere görev çubuğunun
  altına kaymaz, tam kenarında durur.
- Sekmeler taşınabilen, sıralanabilen, çoğaltılabilen ve ayrılabilen hap biçimli düğmelerdir. Ctrl+T,
  Ctrl+W, Ctrl+Tab ve orta tık Explorer'daki gibi çalışır.
- Simgeler, küçük resimler ve dosya türü adları Windows'tan gelir.
- Her Explorer kısayolu aynı şekilde çalışır. Tam liste için F1.
- Sistem, Açık ve Koyu dışında Görünüm > Tema menüsünde Catppuccin (Mocha, Latte), Gruvbox, Everforest,
  Nord, Dracula, Tokyo Night, Rosé Pine ve Solarized var.
- Arayüz İngilizce, Türkçe, Almanca, İspanyolca, Fransızca, Brezilya Portekizcesi, Rusça, Basitleştirilmiş
  Çince ve Japonca. Dil Windows'u izler, Görünüm > Dil menüsünden değiştirilir.
- Çince, Japonca ve Korece dosya adları sistemin CJK yazı tipleriyle görünür.

**Arama ve önizleme**

- Ctrl+K komut paletini açar: yazdıkça dosya arar, her komutu çalıştırır, bir yola veya son klasöre atlar.
  Ctrl+E, Ctrl+F ve F3 de açar; `*` ve `?` joker karakterleri çalışır.
- Boşluk tuşu seçili öğede Hızlı Bakış açar: resimler, PDF'ler, ses ve video satır içinde oynar, metin ve
  Markdown söz dizimi renklendirmesiyle gösterilir, klasörler içeriğini listeler.
- Markdown, PDF ve video önizlemeleri gömülü WebView2 bölmesinde çizilir; bir README veya PDF başka bir
  uygulamada değil, bulunduğunuz yerde açılır.
- Adres çubuğuna `cmd`, `powershell`, `pwsh` veya `wt` yazmak o terminali bulunduğunuz klasörde açar,
  `terminal` Windows Terminal'i açar. `\\wsl$\…` yolları diğer klasörler gibi açılır.
- Git sütunu bir deponun dal durumunu klasörün içinde gösterir.

**Dosyalar**

- Kopyala, kes, yapıştır ve sürükle-bırak Explorer ve masaüstüyle iki yönde çalışır.
- Shift + sağ tık tam Windows bağlam menüsünü açar; 7-Zip, WinRAR ve Gönder buradadır.
- Ctrl+Z yeniden adlandırmayı, taşımayı, kopyalamayı ve Geri Dönüşüm Kutusu'na silmeyi geri alır.
- Kopyalama veya taşıma, dosyalar diske yazıldıktan sonra biter (terminaldeki `sync` gibi). İlerleme
  çubuğu kaybolduğunda USB bellek çıkarılabilir; başka sürücüye taşımada kaynaklar ancak ondan sonra
  silinir.
- Aynı adlı dosya varsa değiştir, atla veya ikisini de sakla diye sorar.
- Klasörler diskte değişiklik olduğunda canlı güncellenir.
- İndirilenler tarihe göre gruplanır (Bugün, Dün, Geçen hafta…). Diğer klasörler Sırala > Grupla ile
  gruplanabilir.
- Windows'ta pencereyi kapatmak uygulamayı arka planda bekletir; yeniden açıldığında pencere ekran
  kartının başlamasını beklemeden hemen görünür. Görünüm > Oturum açılışında arka planda başlat ile
  yeniden başlatma sonrası ilk açılış da hızlı olur.

| Ayrıntılar | Bu bilgisayar |
| --- | --- |
| ![Ayrıntılar görünümü](docs/details.png) | ![Bu bilgisayar](docs/this-pc.png) |

| Açık tema | Türkçe arayüz |
| --- | --- |
| ![Açık tema](docs/light.png) | ![Türkçe arayüz](docs/turkish.png) |

Tür sütunu ve Shift + sağ tık menüsü Windows'tan geldiği için, uygulama İngilizce olsa bile Windows'un
görüntüleme dilinde kalır.

## Linux

Linux sürümü Windows kadar test edilmedi. Görünüm ve kısayollar aynı, farklar şunlar:

- Simgeler masaüstünüzün simge temasından gelir. PNG, JPEG, GIF, WebP ve BMP için küçük resim gösterilir.
- Windows bağlam menüsü yok; Shift + sağ tık ve Shift+F10 uygulamanın kendi menüsünü açar.
- Kopyalanan dosyalar panoya yol listesi olarak gider. Diğer dosya yöneticilerinden kopyalanan yollar ve
  `file://` bağlantıları yapıştırılabilir; başka yöneticilere yapıştırmak çalışmayabilir.
- Dosyaları pencere dışına sürüklemek henüz çalışmıyor; içine bırakmak X11'de çalışır.
- "Bu bilgisayar" `/` ile `/media`, `/run/media` ve `/mnt` altındaki diskleri listeler.
- Özellikler, Genel, Güvenlik ve Ayrıntılar sekmeleri olan kendi penceresini açar; izinler ve varsayılan
  uygulama buradan değiştirilir.
- "Dosya yöneticisinde göster" D-Bus üzerinden kendi dosya yöneticinizi kullanır (Nautilus, Dolphin,
  Nemo ve Thunar destekler).

## Derleme

Rust 1.82 veya daha yenisi gerekir.

```bash
cargo build --release
```

Çalıştırılabilir dosya Windows'ta `target/release/starfile.exe`, Linux'ta `target/release/starfile` olur.
`cargo test` ayrıca çeviri tabloları birbiriyle uyuşmazsa başarısız olur.

Kurulum dosyası için [Inno Setup 6](https://jrsoftware.org/isinfo.php) kurup şunu çalıştırın:

```powershell
ISCC.exe /DAppVersion=0.8.0 packaging\windows\starfile.iss
```

Çıktı `dist\StarFile-Setup-0.8.0.exe` olur. Sürüm iş akışı her etikette aynısını yapar.

## Ayarlar

Ayarlar Windows'ta `%APPDATA%\StarFile\config.json`, Linux'ta `~/.config/StarFile/config.json` içinde
tutulur. Uygulama ilk açılışta eski Swift Explorer kurulumunun ayarlarını (`%APPDATA%\SwiftExplorer`)
kopyalar, böylece ad değişikliğinden sonra tercihleriniz korunur. `STARFILE_LANG` ortam değişkenine bir
dil kodu (`en`, `tr`, `de`, `es`, `fr`, `pt`, `ru`, `zh`, `ja`) vermek dil ayarını geçersiz kılar; eski
kısayollar için `SWIFT_EXPLORER_LANG` de okunur.

## Bilinen sınırlar

- Ayrıntılar görünümünde seçim dikdörtgeni yalnızca satırların altındaki boş alandan başlatılabilir.
- Windows, çok uzun yollu (yaklaşık 260+ karakter) dosyaları Geri Dönüşüm Kutusu'na taşıyamaz; doğrudan
  silinir ve Ctrl+Z geri getiremez.
- Ağ konumları Windows Dosya Gezgini'nde açılır.
- Yükleyici imzasızdır (bkz. [Kurulum](#kurulum)).

## Katkı ve teşekkür

StarFile Explorer, [Swift Explorer](https://github.com/misutesu-desu/swift-explorer) projesinin bir
çatalıdır; dosya yöneticisinin kendisi, kabuk entegrasyonu, komut paleti, hızlı bakış ve arayüzün büyük
bölümü Abdullah Şafer'e aittir. Bu çatal adı, simgeyi, kurulum/güncelleme kanalını ve çerçevesiz pencere
çalışmasını ekler. İki proje de MIT lisanslıdır; özgün telif bildirimi [`LICENSE`](LICENSE) içinde
korunmuştur, ayrıntılar [`NOTICE`](NOTICE) dosyasındadır.

## Lisans

[MIT](LICENSE). Linux derlemesi Microsoft'un
[Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons) simgelerinden birkaçını içerir
([MIT](assets/LICENSE-fluent-icons.txt)). Tam liste için [`NOTICE`](NOTICE).
