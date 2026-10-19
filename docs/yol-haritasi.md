# Yol haritası: eksikler

Bu sürümde eklenenler ve hâlâ eksik olanlar. NetSupport School ile karşılaştırma
öğretmenin derste ihtiyaç duyacağı özelliklere göre yapıldı.

## Bu sürümde düzeltilen / eklenenler

- **Sınıf modu**: sınıf oluşturma, ağda otomatik sınıf bulma, öğrenci küçük resimleri,
  öğretmen ekranını yansıtma, bir öğrencinin ekranını herkese gösterme, ekran kilitleme,
  mesaj, web sitesi açma, söz isteme, uzaktan kontrol, otomatik yeniden bağlanma.
- **Kurulum**: Bireysel / Öğretmen / Öğrenci seçimli Windows kurulum dosyası, sessiz
  kurulum (`/S /ROLE=student`), güvenlik duvarı kuralı, kaldırıcı, otomatik yayın.
- **Kararlılık**:
  - Arayüz kuyruğu (8 olay) dolunca oturum "Input queue full / UI unavailable"
    hatasıyla kapanıyordu; kuyruk büyütüldü, olaylar ayrı iş parçacığında işleniyor.
  - Uzak kontrol pencere küçültülünce duruyordu; girişler artık arayüzden bağımsız.
  - Görüntü çözme arayüz iş parçacığından alındı; yalnızca en yeni kare tutuluyor.
  - Ağ döngüsü her turda tek komut işliyordu (fare/klavye gecikmesi); 32'ye çıkarıldı.
  - Kilit ekranı/UAC gibi kısa ekran yakalama hatalarında paylaşım hemen kapanıyordu;
    artık ~30 saniye boyunca tekrar deneniyor.
  - Her fare hareketinde bütün ekranlar yeniden listeleniyordu; ekran bilgisi önbellekte.
  - Ses oynatıcı açılamazsa her ses paketinde yeniden deneniyordu.
  - Ayar dosyası yarım yazılabiliyordu (atomik yazım), `devices` alanı eksik bir ayar
    dosyası bütün ayarları sıfırlıyordu.
- **Tasarım / ekran boyutu**:
  - En küçük pencere boyutu yoktu; artık 720×500. 900 px altında kenar menüsü
    simgelere küçülüyor.
  - Uzak ekran sola yaslıydı ve büyütülmüyordu; ortalanıyor, pencereye sığdırılıyor,
    "Pencereye sığdır" kapatılınca 1:1 kaydırmalı gösteriliyor.
  - Oturum yan paneli sabit 320 px'ti; yeniden boyutlanabiliyor, tam ekranda gizleniyor.
  - Bazı simgeler (●, ▤, ☾) yazı tipinde olmadığı için kutu olarak görünüyordu.
  - Paylaşım kartında bu bilgisayarın yerel IP adresi gösteriliyor; davet bağlantısı
    127.0.0.1 yerine gerçek adresle oluşuyor; IP'ye port yazılmazsa 7878 ekleniyor.
  - Sohbette Enter ile gönderme, otomatik aşağı kaydırma; dosyayı pencereye
    sürükleyip bırakarak gönderme.
  - İzleyicinin kalite/FPS değişikliği karşı tarafa gönderilmiyordu.
  - Kalite, FPS, port, klasör ve yeniden bağlanma ayarları kaydedilmiyordu.

## Hâlâ eksik olanlar (öncelik sırasıyla)

1. **Gerçek bilgisayarlarda sınıf testi**: protokol ve arayüz otomatik testlerle ve
   tek makinede iki uygulamayla denendi; 20–30 öğrencili gerçek bir sınıf ağında,
   Windows'ta denenmedi. İlk derste 2–3 bilgisayarla deneme yapılmalı.
2. **Öğrencinin uygulamayı kapatmasını / kilidi aşmasını engelleme**: kilit ekranı
   tam ekran ve en üstte açılır ama Alt+Tab, Ctrl+Alt+Del veya uygulamayı kapatma
   engellenmez. NetSupport bunu bir Windows hizmetiyle yapar; bunun için Opendesk'in
   arka plan hizmeti olarak çalışması gerekir.
3. **Video kodlama**: görüntü JPEG kareleriyle gider. Öğretmen ekranı 30 öğrenciye
   aynı anda yansıtılırken 100 Mbit ağda yavaşlayabilir. H.264 veya UDP multicast
   ile tek yayın yapılması gerekir.
4. **Dosya dağıtma / toplama**: öğretmenden bütün sınıfa dosya gönderme ve ödev
   toplama sınıf modunda yok (1'e 1 oturumda var).
5. **Uygulama ve web kısıtlama**: izinli/yasak site ve program listeleri yok.
6. **Sınav / anket / quiz** modülü yok.
7. **Ses**: öğretmenin sesini yayınlama ve öğrenci mikrofonu yok.
8. **Öğrenci listesi / kayıt**: sınıf listesi (beklenen öğrenciler, devamsızlık) ve
   oturum raporu yok; katılan öğrenciler yalnızca o oturumda tutulur.
9. **Bilgisayarları uzaktan kapatma / yeniden başlatma / oturum kapatma**, Wake-on-LAN.
10. **Farklı alt ağlar**: sınıf duyurusu yerel ağ yayınıdır (UDP); öğretmen başka
    VLAN'daysa öğrenci IP adresini elle yazmalı.
11. **macOS** desteği ve Android'de sınıf modu yok.
12. **Kod imzalama**: kurulum dosyası imzasız olduğu için Windows SmartScreen uyarı
    verebilir ("Ek bilgi > Yine de çalıştır"). Okul için kod imzalama sertifikası gerekir.
13. **1'e 1 oturum protokolü**: görüntü parçaları JSON sayı dizisi olarak gidiyor
    (~3 kat fazla veri). Sınıf modu ikili biçim kullanıyor; 1'e 1 oturumun da protokol
    sürümü artırılarak ikili biçime geçmesi gerekir.
