# seenboard-plugins

[English](README.md) · **Türkçe**

[Seenboard](https://github.com/omertahaoztop/seenboard) için plugin'ler. Her plugin ayrı bir süreçtir ve çekirdekle satır başına JSON ile konuşur.

Seenboard'un eski adı focusdeck'ti. Bu plugin'ler protokol 2'yi hedefler, `SEENBOARD_LANG` ile dil alır ve yeni adlarla kurulur; focusdeck dönemindeki (protokol 1) plugin ikilileri uyumlu değildir, yeniden kurulmalıdır.

Plugin'ler kullanıcının yetkisiyle çalışır, kum havuzu yoktur. Yalnız güvendiğin dizinlerden kur.

Plugin'ler seenboard'in `SEENBOARD_LANG` ile ilettiği dili konuşur; doğrudan çalıştırılınca `LC_ALL`/`LC_MESSAGES`/`LANG` kullanılır. İngilizce ve Türkçe hazır gelir, başka her dil İngilizceye düşer.

## Kurulum

```
seenboard plugin install <ad>
```

Çekirdek `index.toml` dizininden makinenin hedefini seçer, ikiliyi indirir ve sha256'sını doğrular. Plugin adları: `vikunja`, `redmine`, `zulip`, `whatsapp`, `gmail`. Plugin'in ayarları `~/.config/seenboard/profiles/<ad>.toml` profilindeki kutunun `config` tablosuna yazılır (`config.toml` yalnız `default_profile`, `registries`, `capture_command`, `language` ve `capture_words` tutar). Adı `_file` ile biten ayarlar gizli bilginin durduğu dosyayı gösterir, izni `0600` olmalı.

```toml
[[box]]
id = "redmine"
title = "Redmine planım"
plugin = "redmine"
row = 0
col = 0
config = { url = "https://redmine.example.com", key_file = "~/.config/seenboard/secrets/redmine" }
```

Aşağıdaki tablolardaki anahtarlar bu `config` tablosuna girer.

Sunucu adresleri `https://` ile başlamalı. Düz `http://` yalnız `localhost` ve loopback adreslerinde kabul edilir, yönlendirmeler izlenmez. Böylece anahtar ya da token şifresiz bağlantıdan geçmez, başka bir sunucuya da gitmez.

## vikunja

Vikunja projesindeki açık görevleri listeler, tamamlar, siler, sütun ya da sıra değiştirir, bırakılan notlardan görev açar.

Profil ayarları:

| Anahtar | Anlam |
|---|---|
| `url` | Vikunja adresi, örneğin `https://vikunja.example.com` (`/api/v1` eklenir) |
| `token_file` | API token'ının durduğu dosya, izni `0600` olmalı |
| `project` | Proje başlığı |
| `bucket` | İsteğe bağlı sütun başlığı, verilirse yalnız o sütun listelenir |
| `from_label_prefix` | İsteğe bağlı, notun gönderenini taşıyan etiketin ön eki. Varsayılan `kimden:`, boş bırakılırsa etiket kullanılmaz |

Proje ve sütun başlığında önce birebir eşleşme aranır, bulunamazsa büyük küçük harf farkı yok sayılır.

Gelen notlarda `from` `<from_label_prefix><ad>` etiketine (varsayılan `kimden:<ad>`), `labels` etiketlere, `links` açıklamadaki bağ paragrafına döner. Yalnız `http`, `https` ve `mailto` bağları bağ olarak yazılır, diğerlerinin yalnız etiketi düz metin olarak kalır. Başka kutudan gelen öğenin kaynağı açıklamaya `seenboard-source: <plugin>:<id>` satırı olarak yazılır. Yeni görevi etiketlemek ya da sütuna taşımak başarısız olursa görev silinir, yeniden deneyince kopya kalmaz. Silme de başarısız olursa hata mesajı geride kalan görevin numarasını verir.

Başka bir vikunja kutusundan bırakılan not, bağı bu sunucuyu gösteriyorsa ve görev orada hâlâ duruyorsa kopyalanmadan taşınır. Değilse yeni görev açılır.

`done` görevi görünümün bitti sütununa taşır. Görünümde bitti sütunu yoksa görev tamamlandı olarak işaretlenir.

## redmine

Bana atanmış açık Redmine işlerini listeler. Eylem yoktur.

Profil ayarları:

| Anahtar | Anlam |
|---|---|
| `url` | Redmine adresi, örneğin `https://redmine.example.com` |
| `key_file` | API anahtarının durduğu dosya, izni `0600` olmalı |

## zulip

Yıldızlı mesajları listeler, `unstar` eylemiyle yıldızı kaldırır. En yeni 100.000 yıldızlı mesajı okur.

Profil ayarları:

| Anahtar | Anlam |
|---|---|
| `url` | Zulip adresi, örneğin `https://zulip.example.com` |
| `email` | Bot ya da kullanıcının e-posta adresi |
| `key_file` | API anahtarının durduğu dosya, izni `0600` olmalı |

## whatsapp

Tek bir WhatsApp sohbetinin mesajlarını bir [GOWA](https://github.com/aldinokemal/go-whatsapp-web-multidevice) REST sunucusundan listeler; "kendime yaz" sohbeti için düşünüldü. Plugin iki istek atar: listelemek için `GET /chat/<jid>/messages`, `done` eylemi için `POST /message/<id>/delete`. GOWA'nın önündeki proxy'nin yalnız bu ikisini geçirmesi yeter. `done` GOWA'dan mesajı senin için silmesini ister (WhatsApp'taki "benden sil"), GOWA mesajı kendi deposundan da çıkarır. Bu işlem geri alınamaz. Arama kayıtları listelenmez. Eksik gelen liste hatadır. Aralıkta 2.000'den fazla mesaj olan sohbet hatadır; `since` aralığı daraltır. Sunucu bilinmeyen sohbeti bildirmez, bu yüzden yanlış yazılmış `jid` boş kutu verir.

Profil ayarları:

| Anahtar | Anlam |
|---|---|
| `url` | GOWA adresi, örneğin `https://gowa.example.com` |
| `user` | GOWA Basic Auth kullanıcı adı |
| `key_file` | GOWA parolasının durduğu dosya, izni `0600` olmalı |
| `jid` | Kendi sohbetinin JID'i, örneğin `15550001111@s.whatsapp.net`; grup JID'leri (`@g.us`) reddedilir |
| `since` | İsteğe bağlı, `YYYY-AA-GG`; bu günden eski mesajlar listelenmez |

```toml
[[box]]
id = "whatsapp"
title = "WhatsApp"
plugin = "whatsapp"
row = 0
col = 1
config = { url = "https://gowa.example.com", user = "seenboard", key_file = "~/.config/seenboard/secrets/gowa.key", jid = "15550001111@s.whatsapp.net", since = "2026-11-01" }
```

## gmail

Yıldızlı Gmail maillerini listeler, salt okunur yetkiyle çalışır. Eylem yoktur. En çok 10.000 yıldızlı maili okur.

Profil ayarları:

| Anahtar | Anlam |
|---|---|
| `client_file` | Google Cloud'dan indirilen OAuth istemci JSON dosyası. `login` için istemci türü Masaüstü uygulaması olmalı |
| `token_file` | Yenileme token'ının yazılacağı ve okunacağı dosya, izni `0600` olmalı |
| `api_url` | İsteğe bağlı, Gmail API adresini değiştirir; varsayılan Google'ın adresidir |

Token'ı bir kez üretmek için:

```
~/.local/share/seenboard/plugins/gmail login --client-file <yol> --token-file <yol> [--hint <e-posta>]
```

`seenboard plugin install gmail` ikiliyi bu yola `gmail` adıyla koyar. Kaynaktan derlediysen aynı komut `cargo run -p seenboard-gmail -- login ...` ya da derlenen `seenboard-gmail login ...` olur. Oturum düşerse plugin'in hata metni `login` komutunu çalışan ikilinin tam yoluyla verir.

Komut tarayıcıda açman için bir adres basar, onaydan sonra token'ı `--token-file` yoluna `0600` izinle yazar. Onayı 5 dakika bekler ve yalnız kendi başlattığı oturum açma adımının yanıtını kabul eder.

## Çeviri

Her plugin metinlerini `plugins/<ad>/locales/` altında tutar. Yeni dil eklemek için `en.ftl` dosyasını `plugins/<ad>/locales/<kod>.ftl` olarak kopyala (örneğin `de.ftl`) ve değerleri çevir. Mesaj kimliklerine ve `{ $ad }` yer tutucularına dokunma. `cargo test`, dil dosyasında kimlik eksikse, fazlaysa ya da yer tutucular `en.ftl` ile uyuşmuyorsa başarısız olur. Bir dilde olmayan mesaj İngilizce gösterilir.

## Sürüm alma

Her plugin kendi etiketiyle çıkar. Önce `plugins/<ad>/Cargo.toml` içindeki sürümü yükselt ve `main`'e gönder, sonra:

```
git tag <ad>-v<sürüm> && git push origin <ad>-v<sürüm>
```

Etiket `<ad>-v<sürüm>` biçiminde olmalı ve paket sürümüyle uyuşmalı, sürüm de semver olmalı. İş akışı iki hedef için ikili derler, GitHub sürümü açar ve `index.toml` dosyasını günceller. Dizin aracı `index.toml` içindekinden eski bir sürümü reddeder. Depoda değişmez sürümler (immutable releases) açık, `*-v*` etiketleri de kural kümesiyle korunuyor. Yayımlanmış sürüm değiştirilemez, etiketi taşınamaz ve silinemez. İş akışı aynı etiket için yeniden çalışınca yayımdaki dosyaları koruyup dizini onlardan günceller. Başarısız bir çalışmadan kalan taslağı yeni derlemenin dosyalarıyla doldurup yayımlar. Her ikili için derleme kanıtı (build provenance attestation) üretilir, `gh attestation verify <dosya> --repo omertahaoztop/seenboard-plugins` ile doğrulanır. Tek push'ta en fazla üç etiket gönder: GitHub, üçten fazla etiket bir arada gönderilince hiçbir iş akışını başlatmaz.

## Yerel deneme

`bash scripts/local-index.sh` ikilileri derler ve `dist/index.toml` dosyasını üretir. Dosyanın yolunu basar, `dist/` git'e girmez.

CI, `rust-toolchain.toml` ile sabitlenen araç zinciriyle `cargo fmt --all --check`, `cargo test --workspace --locked` ve `cargo clippy --workspace --all-targets --locked -- -D warnings` çalıştırır.
