[English](metrics.md) | **Português**

# Métricas

> **Planejado (a partir da fase F1).** Esta página documenta o desenho para que os números signifiquem o que dizem quando existirem. Veja a [spec](spec.pt-BR.md), seção 10, para o detalhe completo.

## Ingestão

O tokred lê `<claude>/projects/**/*.jsonl` de forma incremental (a tabela `ingest_state` guarda o offset e o mtime de cada arquivo) e extrai, das mensagens do assistente: modelo, `input_tokens`, `output_tokens`, `cache_creation_input_tokens` e `cache_read_input_tokens`. As linhas são deduplicadas por id da mensagem + id da requisição, porque o streaming pode repeti-las, e as linhas de subagentes são marcadas. Os nomes dos campos são confirmados num transcript real antes de escrever o ingestor.

## O que o `tokred stats` calcula

| Número | Como |
|---|---|
| Custo real | Soma por modelo de tokens × preço |
| Contrafactual "tudo no opus" | Os mesmos tokens com o preço do opus; a diferença é a **economia por roteamento** |
| Economia das guardas (**estimativa**) | `bytes_saved / 4` como tokens evitados × preço de input do modelo da sessão |
| Taxa de cache de prompt | `cache_read / (cache_read + cache_write + input)` |
| Projeção | Média diária dos últimos 7 e 30 dias × 30 |
| Antes e depois | Custo por sessão antes e depois do primeiro `configure`, quando há transcripts mais antigos |

Tudo que deriva de bytes (`bytes / 4`, caracteres / 4 na auditoria) é sempre rotulado como estimativa.

## Latência adicionada pelo tokred

Os hooks rodam de forma síncrona antes de o Claude Code continuar, então o `tokred stats latency` mostra por hook: execuções, p50, p95, p99 e máximo em ms; totais por sessão e por dia; contagem de `error`, `panic` e `timeout_guard`; e um alerta quando o p95 de um hook passa do orçamento de 50 ms. O tempo de spawn do processo não é visível de dentro do binário, então o `tokred doctor --bench` o mede de fora e soma às durações internas, rotulado como estimativa.

## Relatório HTML

O `tokred metrics` grava um HTML autocontido e offline (CSS/JS inline, gráficos SVG gerados em Rust, tema claro/escuro) em `<dados>/reports/metrics.html`, sempre o mesmo arquivo, de forma atômica, e abre no navegador a menos que se passe `--no-open`. Seções sem dados mostram um aviso de "sem dados ainda" em vez de sumir.

Os preços vêm do JSON público do LiteLLM com fallback embutido; um modelo desconhecido é mapeado para o preço mais recente da família (opus/sonnet/haiku) e sinalizado.
