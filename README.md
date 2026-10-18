# Opendesk 0.2.0

Windows ve Linux için Rust masaüstü; Android için Flutter arayüzü + aynı Rust
bağlantı çekirdeği. Bu sürüm bir önizlemedir. AnyDesk ağı veya istemcileriyle
uyumlu değildir. 0.1 istemcilerinin her iki uçta da 0.2 sürümüne yükseltilmesi gerekir.

## Eklenen 15 başlık

| Başlık | Bu sürümdeki kapsam |
| --- | --- |
| Kolay bağlantı kodu | 100 bit rastgele, 20 karakterli gruplu kod ve özel bağlantı daveti; relay için sunucu adresi bir kez ayarlanır |
| Cihaz listesi/favoriler | Ad, adres ve favoriler kullanıcı ayarlarında saklanır; parolalar saklanmaz |
| Gözetimsiz erişim | Açık etkinleştirme, Argon2 parola + doğrulayıcı uygulama TOTP; gizli anahtarlar OS credential store içinde |
| Wayland | RemoteDesktop/ScreenCast portal izni, PipeWire/GStreamer görüntü ve portal giriş; gerçek compositor testi bekliyor |
| Android | ARM64 APK: bilgisayarı izleme/kontrol etme ve sistem izniyle Android ekranını paylaşma |
| Sistem sesi | Windows WASAPI loopback; Linux monitor kaynağı ve PCM oynatma; izin ayrı yönetilir |
| Görüntü aktarımı | JPEG, ölçekleme, kalite/FPS ve en yüksek genişlik ayarı; H.264/H.265 kullanılmaz |
| Otomatik kalite | Görüntü alma hızına göre kalite/FPS geri bildirimi |
| Canlı monitör değiştirme | Uzak monitör listesi ve oturum sırasında seçme; Wayland kaynağını portal seçer |
| Gelişmiş dosya aktarımı | Dosya/klasör, ilerleme, diskten akış, SHA-256 ve doğrulanmış kısmi dosyadan devam |
| Yeniden bağlantı | İsteğe bağlı, iki rolde de en fazla 5 deneme; yeni onay/TOTP gerekir; STOP iptal eder |
| Tam ekran/klavye | Tam ekran, Esc, geliştirilmiş tuş eşleme, odak kaybında basılı girişleri bırakma |
| İzinler | Ekran, giriş, pano, dosya ve ses ayrı; oturum sırasında iptal edilebilir |
| Kayıt | Görünür kayıt göstergesi, gerçek indexed MJPEG AVI video |
| Kurulum/güncelleme | Kullanıcıya özel kurulum, isteğe bağlı görünür otomatik başlatma; imzalı güncelleme kontrol/indirme/uygulama |

## Windows kullanımı

ZIP paketini tüm dosyalarıyla çıkar. Taşınabilir kullanım için Opendesk.exe;
kurulum için opendesk-setup.exe çalıştır. Kurulum yöneticisi yalnızca Install
düğmesine basılınca dosya kopyalar. Kısayol/otomatik başlatma varsayılan kapalıdır.
Otomatik başlatma pencereyi açar; kendiliğinden dinleme veya erişim izni vermez.

1. Paylaşacak bilgisayarda bağlantı kodunu kopyala; monitör ve izinleri seç.
2. Start listening seç; varsayılan TCP portu 7878.
3. Diğer bilgisayarda aynı kodu ve paylaşanın IP:port adresini gir, Connect seç.
4. Paylaşan bilgisayarda kabul et. Gözetimsiz erişim ayrıca kurulmuşsa parola
   ve yeni TOTP doğrulaması kullanılır.
5. STOP / Disconnect erişimi bitirir ve yeniden bağlanma denemelerini iptal eder.

Windows güvenlik duvarı için kullanılacak ağ kapsamına uygun izin gerekir;
uygulama güvenlik duvarı ayarlarını kendisi değiştirmez.

## İnternet üzerinden, yalnızca kodla (varsayılan)

Ayarlar > Bağlantı yolu > İnternet (yalnızca kod). IP, port yönlendirme veya
kendi sunucun gerekmez: paylaşan taraf Paylaşımı başlat'a basar, diğer taraf
yalnızca 20 karakterli kodu girer. Bağlantı iroh (QUIC) ile kurulur; mümkünse
UDP delik açma ile doğrudan, değilse n0'ın ücretsiz genel relay sunucuları
üzerinden gider. iroh uç noktası koddan türetilir, yani kodu bilmeyen host'u
bulamaz. Noise PSK şifrelemesi, yerel onay ve izinler aynen geçerlidir.
Oturum sırasında durum satırı yolun doğrudan mı relay mi olduğunu ve RTT'yi
gösterir. n0 relay'leri ücretsizdir ama hız sınırlı olabilir ve hizmet garantisi
yoktur. İki ucun da bu sürümü kullanması gerekir. Canlı test:
`cargo test --lib p2p -- --ignored`.

## Relay ve davet

Kendi erişilebilir sunucunda:

```sh
opendesk-relay 0.0.0.0:7879
```

İki uçta Use self-hosted relay seç; aynı sunucu adresini ve bağlantı kodunu gir.
Önce paylaşan taraf bağlanıp relay bekleme durumuna geçsin. Sonra diğer uç
bağlansın. Davet kodu sunucu/adres ve eşleştirme sırrını içerir; özel paylaşılır.
Bu çalışma sırasında halka açık sunucu veya cihaz dizini hizmeti kurulmadı.

## Gözetimsiz erişim

Parola en az 12 byte olmalı. Enrollment ile oluşturulan TOTP anahtarını
Authenticator uygulamana ekle; ilk kodu doğruladıktan sonra özellik etkinleşir.
Aynı TOTP adımı tekrar kullanılamaz; hatalı denemeler geçici kilit uygular.
Windows Credential Manager veya Linux Secret Service gerekir. Kilitli veya
ulaşılamayan kasa erişimi engeller. Opendesk açık ve dinliyor olmalıdır;
arka planda gizli servis kurulmaz. Disable ile yetki kaldırılır.

## Dosya/klasör ve kayıt

Alıcı gelen manifesti ve hedef klasörü ayrıca onaylar. Yarım aktarımı sürdürmek
 için aynı kaynağı yeniden gönder ve aynı alıcı klasörünü seç. Tamamlanmış dosya
üzerine yazılmaz; kısmi önek ve son içerik SHA-256 ile doğrulanır.
Sınırlar: 128 dosya, 240 byte göreli yollar, toplam 1 TiB. Boş klasörler ve
sembolik bağlantılar aktarılmaz. Eski tek dosya/pano yollarının sınırları
16 MiB / 8 KiB olarak kalır. Ayrıntılar docs/transport-v2.md dosyasındadır.

Video kaydı açıkça başlatılır ve kırmızı göstergeyle görünür; AVI olarak
sonlandırılır. JPEG karelerden oluşur, ses kayda eklenmez; değişken kare geliş
hızı nedeniyle gerçek oturum süresiyle video süresi farklı olabilir.

## Linux ve Wayland

Ubuntu kaynak derleme bağımlılıkları:

```sh
sudo apt-get install build-essential pkg-config libx11-dev libxrandr-dev libxcb1-dev libdbus-1-dev libxkbcommon-dev libxdo-dev libwayland-dev libasound2-dev
cargo build --release --bins
```

Wayland ekran paylaşımı ve giriş için xdg-desktop-portal, masaüstüne uygun
portal backend'i, PipeWire ve GStreamer pipewiresrc/videoconvert/jpegenc/fdsink
pluginleri gerekir. Ekran/giriş seçimini işletim sistemi izin penceresi yapar.
Linux sesi için PulseAudio/PipeWire monitor kaynağı ve ffmpeg gerekir.
Linux paketinin runtime açıklamasını oku; Ubuntu 26.04 derlemesi her dağıtımda
çalışacak evrensel paket değildir. Eski glibc dağıtımlarında kaynaktan derle.

## Android önizleme

APK ARM64, Android 7.0/API24 ve üzeri içindir. Flutter arayüzü aynı Rust
Noise v2 çekirdeğini kullanır. MediaProjection ekran paylaşımında sistem
izni ve görünür durdurma bildirimi zorunludur. Android host bu sürümde yalnızca
ekran paylaşır; host kontrolü, pano, ses ve dosya aktarımı kapalıdır. Android
viewer bilgisayarı kontrol edebilir, metin/pano/sohbet ve monitör seçimi sunar.
Geliştirme anahtarıyla imzalanmış önizlemedir; mağazaya yayınlanmamıştır.
Gerçek telefonda MediaProjection/bağlantı testi henüz yapılmadı.

## İmzalı güncellemeler

Varsayılan güncelleme sunucusu yoktur. Settings/Updates bölümünde kendi HTTPS
manifest adresini tanımla. Check -> Download -> Apply and restart adımları
kullanılır. Ed25519 imzası, platform/sürüm, boyut ve SHA-256 doğrulanır;
downgrade reddedilir. Sadece localhost testlerinde HTTP kullanılabilir.
İmza aracı ve yayın akışı docs/releases.md içinde açıklanır. Kaynak projeye
ve paketlere özel imza anahtarı dahil edilmez. Kurulum/güncelleme bu geliştirme
çalışmasında gerçek kullanıcı kurulumuna uygulanmadı.

## Doğrulama

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo run --bin opendesk -- --self-test
cargo run --bin opendesk -- --smoke-test
cargo run --bin opendesk -- --smoke-audio
```

Doğrulama sonuçları docs/validation.md ve docs/cross-os-validation.md içindedir.
Kaynak derlemesi veya sentetik ağ testi, gerçek iki cihazın bütün ekran/giriş,
Wayland veya Android davranışlarını doğrulamış sayılmaz. AnyDesk ile tüm
özelliklerde eşitlik veya üretim kullanımı için tam kabul testi iddiası yoktur.

## Tasarım

Claude Design ZIP'indeki arayüz Rust/egui ve Android/Flutter'a uyarlandı.
Açık/koyu tema, Türkçe/İngilizce, IBM Plex fontları, pencere logosu, Bağlan /
Cihazlar / Ayarlar ve oturum panelleri gerçek uygulama verilerini kullanır.
Referans dosyaları design-reference/, uygulama görünüm örnekleri docs/previews/
içindedir. Ayrıntılar docs/design-integration.md içinde bulunur.
