# focusdeck-plugins

[English](README.md) · **Türkçe**

focusdeck için plugin'ler. Her plugin ayrı bir süreçtir ve çekirdekle satır başına JSON ile konuşur.

Plugin'ler kullanıcının yetkisiyle çalışır, kum havuzu yoktur. Yalnız güvendiğin dizinlerden kur.

Plugin'ler focusdeck'in `FOCUSDECK_LANG` ile ilettiği dili konuşur; doğrudan çalıştırılınca `LC_ALL`/`LC_MESSAGES`/`LANG` kullanılır. İngilizce ve Türkçe hazır gelir, başka her dil İngilizceye düşer.

## Kurulum

```
focusdeck plugin install <ad>
```

Çekirdek `index.toml` dizininden makinenin hedefini seçer, ikiliyi indirir ve sha256'sını doğrular. Plugin adları: `vikunja`, `redmine`, `zulip`, `gmail`. Plugin'in ayarları `~/.config/focusdeck/profiles/<ad>.toml` profilindeki kutunun `config` tablosuna yazılır (`config.toml` yalnız `default_profile`, `registries` ve `capture_command` tutar). Adı `_file` ile biten ayarlar gizli bilginin durduğu dosyayı gösterir, izni `0600` olmalı.

```toml
[[box]]
id = "redmine"
title = "Redmine planım"
plugin = "redmine"
row = 0
col = 0
config = { url = "https://redmine.example.com", key_file = "~/.config/focusdeck/secrets/redmine" }
```

Aşağıdaki tablolardaki anahtarlar bu `config` tablosuna girer.

## vikunja

Vikunja projesindeki açık görevleri listeler, tamamlar, siler, sütun ya da sıra değiştirir, bırakılan notlardan görev açar.

Profil ayarları:

| Anahtar | Anlam |
|---|---|
| `url` | Vikunja adresi, örneğin `https://vikunja.example.com` (`/api/v1` eklenir) |
| `token_file` | API token'ının durduğu dosya, izni `0600` olmalı |
| `project` | Proje başlığı |
| `bucket` | İsteğe bağlı sütun başlığı, verilirse yalnız o sütun listelenir |

Gelen notlarda `from` `kimden:<ad>` etiketine, `labels` etiketlere, `links` açıklamadaki bağ paragrafına döner. Başka kutudan gelen öğenin kaynağı açıklamaya `focusdeck-source: <plugin>:<id>` satırı olarak yazılır.

## redmine

Bana atanmış açık Redmine işlerini listeler. Eylem yoktur.

Profil ayarları:

| Anahtar | Anlam |
|---|---|
| `url` | Redmine adresi, örneğin `https://redmine.example.com` |
| `key_file` | API anahtarının durduğu dosya, izni `0600` olmalı |

## zulip

Yıldızlı mesajları listeler, `unstar` eylemiyle yıldızı kaldırır.

Profil ayarları:

| Anahtar | Anlam |
|---|---|
| `url` | Zulip adresi, örneğin `https://zulip.example.com` |
| `email` | Bot ya da kullanıcının e-posta adresi |
| `key_file` | API anahtarının durduğu dosya, izni `0600` olmalı |

## gmail

Yıldızlı Gmail maillerini listeler, salt okunur yetkiyle çalışır. Eylem yoktur.

Profil ayarları:

| Anahtar | Anlam |
|---|---|
| `client_file` | Google Cloud'dan indirilen OAuth istemci JSON dosyası |
| `token_file` | Yenileme token'ının yazılacağı ve okunacağı dosya, izni `0600` olmalı |
| `api_url` | İsteğe bağlı, Gmail API adresini değiştirir; varsayılan Google'ın adresidir |

Token'ı bir kez üretmek için:

```
~/.local/share/focusdeck/plugins/gmail login --client-file <yol> --token-file <yol> [--hint <e-posta>]
```

`focusdeck plugin install gmail` ikiliyi bu yola `gmail` adıyla koyar. Kaynaktan derlediysen aynı komut `cargo run -p focusdeck-gmail -- login ...` ya da derlenen `focusdeck-gmail login ...` olur. Oturum düşerse plugin'in hata metni `focusdeck-gmail login` der, bu ikilinin kaynaktaki adıdır; kurulu ikili için yukarıdaki yolu kullan.

Komut tarayıcıda açman için bir adres basar, onaydan sonra token'ı `--token-file` yoluna `0600` izinle yazar.

## Çeviri

Her plugin metinlerini `plugins/<ad>/locales/` altında tutar. Yeni dil eklemek için `en.ftl` dosyasını `plugins/<ad>/locales/<kod>.ftl` olarak kopyala (örneğin `de.ftl`) ve değerleri çevir. Mesaj kimliklerine ve `{ $ad }` yer tutucularına dokunma. `cargo test`, dil dosyasında kimlik eksikse, fazlaysa ya da yer tutucular `en.ftl` ile uyuşmuyorsa başarısız olur. Bir dilde olmayan mesaj İngilizce gösterilir.

## Sürüm alma

Her plugin kendi etiketiyle çıkar. Önce `plugins/<ad>/Cargo.toml` içindeki sürümü yükselt ve `main`'e gönder, sonra:

```
git tag <ad>-v<sürüm> && git push origin <ad>-v<sürüm>
```

Etiket `<ad>-v<sürüm>` biçiminde olmalı ve paket sürümüyle uyuşmalı. İş akışı iki hedef için ikili derler, GitHub sürümü açar ve `index.toml` dosyasını günceller. Tek push'ta en fazla üç etiket gönder: GitHub, üçten fazla etiket bir arada gönderilince hiçbir iş akışını başlatmaz.

## Yerel deneme

`bash scripts/local-index.sh` ikilileri derler ve `dist/index.toml` dosyasını üretir. Dosyanın yolunu basar, `dist/` git'e girmez.
