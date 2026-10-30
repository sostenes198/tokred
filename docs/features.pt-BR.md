[English](features.md) | **Português**

# Ligar e desligar funcionalidades

Cada parte opcional do tokred se liga e desliga com um comando ou uma linha do `tokred.toml`. Esta página lista todas, o que cada uma muda e quando passa a valer. Os comandos estão em [comandos](commands.pt-BR.md) e as chaves em [configuração](configuration.pt-BR.md).

## Resumo

| Funcionalidade | Ligar | Desligar | Passa a valer |
|---|---|---|---|
| Hooks e agentes | `tokred configure` | `tokred unconfigure` | Depois de reiniciar o Claude Code |
| Um hook | a chave de `[modes]` em `warn` ou `enforce` | a chave em `"off"` | Na próxima vez que o hook rodar |
| Guarda de troca de modelo | `tokred model-guard on` (pergunta) ou `warn` (diz quanto custa) | `tokred model-guard off` | Na próxima vez que o hook rodar |
| Funcionalidades do proxy | `tokred proxy enable` | `tokred proxy disable` | Na hora |
| O processo do proxy | `tokred proxy start` | `tokred proxy stop` | Na hora; `tokred proxy restart` o reinicia |
| Iniciar no login | `tokred proxy service install` | `tokred proxy service uninstall` | No próximo login |
| Texto das conversas no dashboard | `tokred proxy enable --record-content true` | `tokred proxy enable --record-content false` | Na hora; um proxy em execução é reiniciado |
| Roteamento de modelos | `tokred routing on` | `tokred routing off` | Na hora; um proxy em execução é reiniciado |
| Compressão reversível | `tokred compression on` | `tokred compression off` | Depois de `tokred proxy restart` e de reiniciar o Claude Code |
| Sugestões | `[suggest] channel` em `stats`, `statusline` ou `message` | `channel = "off"` | Depois de `tokred proxy restart` |
| Avisos de atualização | `[update] mode = "notify"` | `mode = "off"`, ou `TOKRED_NO_UPDATE_CHECK=1` | No próximo comando |
| Chamadas de rede automáticas | remova `TOKRED_OFFLINE` | `TOKRED_OFFLINE=1` | No processo que a tem definida |
| O wrapper do `claude` | o padrão do instalador | instalar com `--no-alias`; `command claude` o ignora numa execução | No próximo shell |

O `tokred uninstall` remove tudo isso. Nenhuma chave desta página muda o modelo do loop principal.

## Hooks e agentes

O `tokred configure` adiciona os hooks do tokred e os seus seis agentes às configurações e à pasta de agentes do Claude Code; o `tokred unconfigure` os retira de novo (`--purge` também apaga a pasta de dados do tokred). Nenhum dos dois mexe no proxy. Para silenciar um hook sem remover nada, coloque o modo dele em `"off"` no `tokred.toml`:

```toml
[modes]
guard_read = "off"
```

O que cada hook faz, e o que significam `warn` e `enforce`, está em [hooks e agentes](hooks-and-agents.pt-BR.md).

## O proxy

O `tokred proxy enable` é a chave geral: salva a porta e se o texto das conversas é gravado, inicia o proxy e aponta o Claude Code para ele. O `tokred proxy disable` desliga isso e remove o endereço do tokred das configurações do Claude Code; um proxy em execução continua rodando até o `tokred proxy stop`. As funcionalidades abaixo só importam enquanto o proxy roda. A medição fica sempre ligada; as outras têm chave própria:

- **Roteamento** passa requisições de subagentes e auxiliares para um modelo mais barato quando é seguro. Começa em `observe` (registra o que faria, não muda nada). Veja [roteamento](routing.pt-BR.md).
- **Compressão** troca resultados grandes de ferramentas por uma prévia que o modelo expande com `tokred_retrieve`. Começa em `observe` (só mede). Veja [proxy](proxy.pt-BR.md).
- **Sugestões** recomendam um modelo mais barato para o loop principal; nunca o trocam.
- **Texto das conversas** é o que o [dashboard](dashboard.pt-BR.md) mostra de cada requisição e resposta. Fica no banco do tokred nesta máquina.

## Roteamento: confira antes de ligar

O roteamento não se liga sozinho e não aprende: fica em `observe` até você rodar `tokred routing on`. Pergunte antes o que ele teria feito num período:

```sh
tokred routing status --since 14d
```

A resposta diz quantas requisições o roteamento teria alterado, quanto isso teria economizado e quanto dá em um mês. Quando nenhuma seria alterada, ligar não muda nada por enquanto. O `tokred doctor` e o `tokred compression status` dão o mesmo tipo de resposta para o roteamento e para a compressão enquanto estão em `observe`, com base nos últimos 7 dias de medições (todas estimativas). Quando uma delas claramente vale a pena (roteamento: mais de cerca de US$ 1 em 7 dias; compressão: mais de 5% dos tokens de entrada), o tokred também avisa uma vez por semana, numa única linha no seu chat que só você vê; `[suggest] savings_hint = false` desliga essa linha.

## Aplicar uma mudança no proxy

O proxy lê as configurações quando inicia. O `tokred routing on` e o `off` reiniciam por você um proxy em execução; depois de editar o `tokred.toml` à mão, ou depois do `tokred compression on`, reinicie:

```sh
tokred proxy restart
```

O `restart` para o proxy e o inicia de novo (um proxy que não estava rodando apenas é iniciado). Ele se recusa enquanto as funcionalidades do proxy estiverem desligadas.
