[English](commands.md) | **Português**

# Comandos

Implementados na fase F0: `configure`, `unconfigure`, `doctor` e o `hook` interno. Os demais comandos da spec chegam com suas fases: `update`, `stats`, `metrics`, `pricing`, `cache` (F1), `audit` (F2).

Os três comandos aceitam `--claude-dir <path>`.

## `tokred configure [--claude-dir <path>] [--yes] [--force]`

Integra o tokred com o Claude Code.

1. Encontra a pasta do Claude Code, nesta ordem: `--claude-dir`, o valor salvo no `tokred.toml` (`[claude] dir`), `$CLAUDE_CONFIG_DIR`, `~/.claude` (`%USERPROFILE%\.claude` no Windows). Uma pasta é válida quando existe e contém `settings.json` ou `projects/`. Um `--claude-dir` explícito que não seja válido é erro. Se nada for encontrado, um terminal interativo é perguntado pelo path; senão o comando falha e sugere `--claude-dir`.
2. Detecta o rtk e informa que os dois vão coexistir.
3. Mostra o **plano**: hooks a adicionar (`+`), atualizar (`~`) ou manter (`=`), agentes a criar, atualizar ou deixar como estão, e o backup que será feito. Pede confirmação; `--yes` pula a pergunta (obrigatório quando o stdin não é um terminal).
4. Aplica: cria a pasta de dados, o banco e o `tokred.toml`, faz o merge do `settings.json` (backup em `settings.json.tokred.bak`, mantendo só o último) e instala os agentes.

Rodar de novo é seguro: o resultado é o mesmo. Se o `settings.json` não for um JSON válido, o comando aborta e **não escreve nada**.

`--force` sobrescreve os arquivos de agente mesmo que você os tenha editado (sem ele, sua versão é mantida e a nova vai para `tokred-x.md.new`).

## `tokred unconfigure [--claude-dir <path>] [--purge]`

Remove os hooks do tokred do `settings.json` e os arquivos de agente que o tokred instalou e que você não editou. Agentes editados são mantidos e informados. Arquivos sem o prefixo `tokred-` nunca são tocados. `--purge` também apaga a pasta de dados; ele se recusa a fazer isso se a pasta não contiver `tokred.db` ou `tokred.toml`.

## `tokred doctor [--claude-dir <path>] [--json]`

Mostra a versão do tokred, o path do binário, as pastas do Claude Code e de dados, a versão do schema do banco, o estado da config, a detecção do rtk, uma tabela de hooks (`ok`, `missing`, `outdated`, ou path de binário que não existe mais), uma tabela de agentes (`current`, `outdated`, `edited by you`, `missing`, `.new` pendente) e uma lista de problemas e observações.

Com `--json` imprime um único objeto JSON:

```json
{
  "tokred_version": "0.1.0",
  "binary": "/opt/homebrew/bin/tokred",
  "binary_on_path": true,
  "data_dir": "…",
  "database": true,
  "schema_version": 1,
  "config": true,
  "rtk_binary_on_path": true,
  "install": {
    "claude_dir": "…",
    "settings_exists": true,
    "settings_valid": true,
    "hooks": [{ "hook": "guard-read", "event": "PreToolUse", "state": "ok", "binary_missing": false }],
    "agents": [{ "file": "tokred-dev.md", "state": "current", "pending_new": false }],
    "rtk_hooks_detected": true,
    "issues": [],
    "notes": []
  }
}
```

O `doctor --bench` (latência dos hooks) chega na fase F3.

## `tokred hook <evento>` (interno)

Chamado pelo Claude Code, nunca à mão (exceto para testes, veja [desenvolvimento](development.pt-BR.md)). Lê JSON do stdin e escreve JSON no stdout. Eventos: `route`, `enforce`, `guard-read`, `guard-search`, `session-check`, `session-start`. **Sempre sai com código 0**, mesmo para evento desconhecido ou entrada inválida. Na fase F0 todo hook é um stub que não imprime nada.
