# focusdeck-plugins

focusdeck için plugin'ler. Her plugin ayrı bir süreçtir ve çekirdekle satır başına JSON ile konuşur.

Plugin'ler kullanıcının yetkisiyle çalışır, kum havuzu yoktur. Yalnız güvendiğin dizinlerden kur.

## vikunja

Vikunja projesindeki açık görevleri listeler, tamamlar, siler, sütun ya da sıra değiştirir, bırakılan notlardan görev açar.

Profil ayarları:

| Anahtar | Anlam |
|---|---|
| `url` | Vikunja adresi, örneğin `https://vikunja.example` (`/api/v1` eklenir) |
| `token_file` | API token'ının durduğu dosya, izni `0600` olmalı |
| `project` | Proje başlığı |
| `bucket` | İsteğe bağlı sütun başlığı, verilirse yalnız o sütun listelenir |

Gelen notlarda `from` `kimden:<ad>` etiketine, `labels` etiketlere, `links` açıklamadaki bağ paragrafına döner. Başka kutudan gelen öğenin kaynağı açıklamaya `focusdeck-source: <plugin>:<id>` satırı olarak yazılır.

## Yerel deneme

`bash scripts/local-index.sh` ikilileri derler ve `dist/index.toml` dosyasını üretir. Dosyanın yolunu basar, `dist/` git'e girmez.
