[English](decisions.md) | **Português**

# Registro de decisões

Formato: data, contexto, decisão, consequências. Onde a spec diverge da documentação oficial do Claude Code, a documentação vence e a divergência fica registrada aqui.

## 2026-10-28 — A ferramenta de subagentes é `Agent`, matcher `Agent|Task`

- **Contexto:** a spec manda confirmar se a ferramenta de subagentes é `Task` ou `Agent` (seção 7.2).
- **Decisão:** segundo a documentação de sub-agents a ferramenta é `Agent`, renomeada de `Task` no Claude Code v2.1.63, com `Task(...)` ainda aceito como alias. O hook `enforce` usa o matcher `Agent|Task` (a sintaxe de matcher aceita nomes exatos separados por `|`).
- **Consequências:** funciona em versões antigas e novas do Claude Code.

## 2026-10-28 — O `timeout` dos hooks é em segundos

- **Contexto:** a documentação de hooks define `timeout` em segundos (padrão 600 para hooks de comando).
- **Decisão:** todo hook do tokred é registrado com `timeout: 5`, como no exemplo da spec.

## 2026-10-28 — Comandos de hook do tokred são reconhecidos estruturalmente, não pela substring `tokred hook `

- **Contexto:** a seção 5.1 da spec diz que o `command` de uma entrada do tokred "contém o marcador `tokred hook `", mas o próprio exemplo é `"/opt/homebrew/bin/tokred" hook guard-read`: a aspa de fechamento fica entre `tokred` e `hook`, então essa substring nunca aparece nas entradas que o próprio tokred grava. Uma implementação literal jamais encontraria (e portanto jamais substituiria ou removeria) as próprias entradas.
- **Decisão:** um comando é do tokred quando executa um executável chamado `tokred` ou `tokred.exe` (opcionalmente entre aspas, precedido por separador de path, aspa, espaço ou início da string) seguido de ` hook <nome>`.
- **Consequências:** idempotência e unconfigure funcionam com paths entre aspas, paths do Windows com espaço e `tokred hook x` solto. Um wrapper como `bash -c 'tokred hook x'` deliberadamente não é reconhecido.

## 2026-10-28 — Os rótulos do formato de retorno ficam em inglês

- **Contexto:** o formato de retorno da spec usa `RESULTADO/ARQUIVOS/PENDÊNCIAS`, mas a seção 0 exige templates de agentes em inglês.
- **Decisão:** os rótulos são `RESULT`, `FILES` e `PENDING`.

## 2026-10-28 — Nomes dos arquivos de documentação em minúsculas

- **Contexto:** a spec diz `docs/DECISIONS.md` na seção 0 e `docs/decisions.md` na seção 16.2.
- **Decisão:** minúsculas, com o par `.pt-BR.md`. A tradução da spec (`docs/spec.md`) mantém o texto original.

## 2026-10-28 — Hooks são stubs fail-open na F0

- **Contexto:** a F0 registra os hooks da seção 7, mas a lógica deles chega nas fases F3 a F5; quem configurar no meio do caminho nunca pode ser prejudicado.
- **Decisão:** `tokred hook <evento>` consome o stdin e não imprime nada. O `hook` é despachado **antes** do parse de argumentos e sempre sai com 0, mesmo para evento desconhecido, porque o Claude Code trata o código 2 como "bloquear".

## 2026-10-28 — A versão mínima do Rust é 1.89

- **Contexto:** a edição 2024 exige 1.85, mas o `rusqlite` 0.41 declara 1.89.
- **Decisão:** `rust-version = "1.89"`, verificado com `cargo +1.89 check --locked` localmente e no CI.

## 2026-10-28 — Pasta de dados via `BaseDirs`, não `ProjectDirs`

- **Contexto:** a spec lista as pastas de dados por SO e cita `directories::ProjectDirs`. No Windows o `ProjectDirs` acrescenta `\data` (`%APPDATA%\tokred\data`), o que não bate com o `%APPDATA%\tokred` da spec.
- **Decisão:** usar `BaseDirs::data_dir().join("tokred")`, que dá exatamente os três paths da spec.

## 2026-10-28 — `option-ext` (MPL-2.0) permitido por exceção por crate

- **Contexto:** `directories` → `dirs-sys` → `option-ext`, licenciado MPL-2.0, que não está na lista de permitidas da spec (MIT, Apache-2.0, BSD, ISC, Unicode, Zlib).
- **Decisão:** manter o `directories` (escolha da spec) e adicionar uma exceção do `cargo-deny` só para esse crate. A MPL-2.0 é copyleft por arquivo e o crate é usado sem modificação.

## 2026-10-28 — Remote do git mantido como o alias SSH existente

- **Contexto:** a spec diz `git@github.com:sostenes198/tokred.git`; o repositório já tinha `git@github.com-pessoal:sostenes198/tokred.git`.
- **Decisão:** manter o remote existente (escolha do autor: usar a configuração de SSH da máquina dele).

## 2026-10-28 — Detalhes de `configure` / `unconfigure` que a spec não fixa

- `--claude-dir` é aceito por `configure`, `unconfigure` e `doctor` (a spec o lista só para `configure`); um path explícito porém inválido é erro e não cai para os outros candidatos.
- `configure` também aceita `--force` (a spec o cita para o `update`).
- Um `settings.json` em branco vale `{}`; JSON inválido aborta sem escrever nada, sem backup e sem criar a pasta de dados.
- O `settings.json` só é regravado quando o resultado do merge difere, e é re-serializado com indentação de 2 espaços (a ordem das chaves é preservada, a formatação original não). O backup guarda o texto anterior exato.
- Limite do round-trip: um array de evento ou objeto `hooks` que **já estava vazio** e no qual o tokred fez merge é removido pelo `unconfigure`, porque "esvaziado pela nossa remoção" não se distingue de "vazio desde o início". Contêineres vazios que o tokred não tocou são preservados.
- Se o `settings.json` não existia, o `unconfigure` deixa `{}`.
- O `unconfigure` mantém os agentes que você editou (e, quando o banco não existe mais, todos os agentes, pois a origem deles não pode ser verificada).
- `--purge` se recusa a apagar uma pasta que não tenha `tokred.db` nem `tokred.toml`.
- O banco só é criado depois que o plano é confirmado.
- O `tokred.toml` só é regravado (sem comentários) quando faltavam chaves; caso contrário fica byte a byte igual.
- `doctor --bench`, sincronização de preços e `tokred metrics` não fazem parte da F0 e ficam fora do início rápido até suas fases.
