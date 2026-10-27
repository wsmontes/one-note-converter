# Plano: conversor completo OneNote → Markdown

Data: 2026-10-26. Baseline e números abaixo foram **medidos** nesta máquina, não estimados
(comandos e corpus no fim).

## 1. Definição de pronto

Um conversor é "completo" quando, para qualquer entrada suportada, cumprir tudo:

1. **Entradas**: `.one` (revision store plano e o empacotado em `[MS-FSSHTTPB]`), `.onetoc2`
   (índice do notebook) e `.onepkg` (arquivo do notebook), além de pastas de notebook exportadas.
2. **Saída 1:1**: uma árvore Markdown espelhando Grupo de seções → Seção → Página, com nomes
   saneados e colisões resolvidas de forma estável.
3. **Conteúdo**: texto com formatação por *run* (negrito, itálico, sublinhado, riscado, cor,
   highlight, link inline), listas numeradas/marcadas com nível, tabelas (célula com texto e imagem),
   imagens, anexos, quebras e parágrafos vazios significativos.
4. **Metadados**: título, data de criação/modificação, autores e tags em front matter opcional.
5. **Sem perda silenciosa**: nada de objeto suportado desaparecer sem aviso; conteúdo cifrado é
   **omitido com aviso explícito** e nunca decodificado como se fosse texto. Cada lacuna conta em
   `--report` (JSON) e no stderr; todo objeto não renderizado tem tipo e motivo registrados.
6. **Invariantes** (já valem hoje, não podem regredir): padding de property set ∈ `0..7`, zero
   referências pendentes em arquivo íntegro, zero pânico em entrada corrompida, `cbExpectedFileLength`
   conferido.
7. **Determinismo**: mesma entrada + mesmas flags ⇒ saída byte a byte idêntica (ordenação estável).

## 2. Baseline medido

Corpus: 34 `.one` + 20 `.onetoc2` de `msiemens/onenote.rs` (exports Windows/OneDrive, Joplin, ink,
alemão) e 10 documentos de teste do Apache Tika.

| Situação | Arquivos | Observação |
| --- | --- | --- |
| Revision store plano (`{109ADD3F}`, `guidLegacyFileVersion` nulo) | 15 | **14 convertem** |
| Package store `[MS-FSSHTTPB]` (`{638DE92F}`) | 26 | rejeitados com mensagem correta |
| `.onetoc2` do corpus | 20 | 17 package store, 3 revision store (já legíveis) |
| Revisões cifradas (`odcsDefault == 0x0002`) | 0 de 114 | gate implementado; verificado por teste sintético |
| Ponteiros Git-LFS / fixture corrompido / `.onetoc2` avulso | 3 | rejeitados |

Nos 14 convertidos: 24 páginas, 189 blocos de texto, 71 assets, 0 objetos pulados, 0 propsets
parciais, 0 padding malformado. Inventário de objetos (soma do corpus):

| Objeto | Qtde | Hoje |
| --- | --- | --- |
| `jcidOutlineElementNode` | 830 | renderizado (parágrafo) |
| `jcidRichTextOENode` | 710 | texto, mas só estilo de parágrafo |
| `jcidParagraphStyleObject` | 301 | usados apenas `Bold`/`Italic`/`Underline` |
| `jcidTableCellNode` / `TableRow` / `TableNode` | 157 / 61 / 30 | tabela OK |
| `jcidImageNode` | 106 | imagem + alt OK |
| `jcidNumberListNode` | 72 | saem como bullet, sem numeração/nível fiéis |
| `jcidEmbeddedFileNode` | 1 | link + bytes OK |
| `0x00060014` (ink container) | 198 | ignorado |
| `0x00020047` / `0x0002003b` | 556 / 234 | tipos sem nome mapeado (metadados) |
| `0x00120043` (`NoteTagSharedDefinition`) | 5 | tags ignoradas |

## 3. Milestones

Cada milestone é entregável isolado, com teste de aceitação sobre arquivo nomeado do corpus.
Tamanho de referência: este crate tem **3 568 linhas** em `src/`; a implementação de referência tem
2 928 linhas de FSSHTTPB (2 070 + 858) e 3 652 de parser desktop — logo M1 é ~do tamanho de todo o
resto junto.

| Milestone | Tamanho estimado | Bloqueia |
| --- | --- | --- |
| M1 FSSHTTPB | ~2 000–3 000 linhas | 26 `.one` + 17 `.onetoc2` |
| M2 formatação por run | ~300–500 | qualidade, não cobertura |
| M3 listas | ~150–250 | qualidade |
| M4 notebook (TOC + CAB) | ~400–700 (+~300 p/ CAB) | árvore completa |
| M5 ink/tags/metadados | ~200–400 | completude |
| M6 sujeira (log/hashed) | ~200–300 | confiabilidade |
| M7 robustez/perf/report | ~150–300 | gate de qualidade |

### M1 — Desembrulhar `[MS-FSSHTTPB]` (P0, maior ganho)

**Por quê**: 26 dos 44 `.one` e 17 dos 20 `.onetoc2` do corpus. O payload é o **mesmo modelo de
objeto** (confirmado em `onenote.rs`: `OneStorePackaging::parse` → `DataElementPackage` →
`parse_store`), então o parser atual é reaproveitado depois do desembrulho.

**Formato**: `[MS-ONESTORE] 2.8.1` (cabeçalho de packaging: 4 GUIDs, `u32` zero, `ObjectHeader` 32
bits tipo `OneNotePackaging` 0x7A, `storage_index`, `cell_schema`, `DataElementPackage`) e
`[MS-FSSHTTPB]` (stream objects, cells, data elements).

**Passos**:
1. `src/package.rs`: detectar o container, validar GUIDs, ler `ObjectHeader`, `ExGuid`, `cell_schema`.
2. `src/fsshttpb/`: leitor de stream objects (32/16/8 bits), `DataElementPackage`, cells.
3. Tradução para o modelo atual: object spaces, revision manifests, grupos, declarações e property
   sets a partir dos data elements — o mesmo `store::Store`, sem bifurcar o renderizador.
4. Reusar `find`/`inspect` para validar contagem de objetos por tipo contra o corpus.

**Aceitação**: `New Section 1.one` (264 KB) converte com páginas e texto comparáveis ao Joplin/Tika;
`Open Notebook.onetoc2` passa a ser lido; 0 objetos pulados nos 26 arquivos; invariantes mantidos.

**Critério de corte**: se M1 passar de ~3 000 linhas novas sem que os **3 primeiros `.one`
empacotados** (New Section 1, New Section Group/New Section 1, OneNote_RecycleBin) convertam com
texto comparável ao mesmo arquivo lido pela referência, parar, manter a mensagem de rejeição atual e
entregar o recorte desktop-only documentado. Não seguir "mais um pouco" sem essa evidência.

**Risco**: compressão deflate dentro dos data elements. Os 4 primeiros arquivos amostrados **não**
mostram cabeçalho zlib, então talvez não haja compressão; se houver, é a primeira decisão aberta
(ver §6).

### M2 — Formatação por run (P0 para "completo")

**Por quê**: 301 objetos de estilo e 710 nós de rich text; hoje só o estilo de parágrafo é aplicado.
Um parágrafo com negrito no meio sai todo plano.

**Formato**: `TextRunIndex` (0x1C001E12, CPs), `TextRunFormatting` (0x24001E13, array de property
sets), `TextRunDataObject`/`TextRunData` (0x24003458 / 0x40003499), `ParagraphStyle` apontando para
`jcidParagraphStyleObject` com `Bold` 0x1C04, `Italic` 0x1C05, `Underline` 0x1C06, `Strikethrough`
0x1C07, `FontColor` 0x1C0C, `Highlight` 0x1C0D, `FontSize` 0x1C0B, `Font` 0x1C0A.

**Passos**: fatiar o texto UTF-16 pelos CPs já parseados, casar cada run com o objeto de estilo,
emitir `**`, `*`, `~~`, `<u>`, e cor/highlight como `<span style>` ou nota de rodapé (decisão §6).

**Aceitação**: página "OneNote Basics" mantém tabela e formatação; novo teste de fixture com
negrito/itálico no meio da frase comparando com o texto esperado do Tika; contagem de `**` > 0 nos
arquivos com estilo `Bold` verdadeiro.

### M3 — Listas fiéis

**Por quê**: 72 `jcidNumberListNode`, hoje todos bullets sem nível.

**Formato**: `jcidNumberListNode` (0x00060012) + `NumberListFormat` (0x1C001C1A; `0xFFFD` marca
numeração) e `ListRestart`, `ListSpacingMu`, `ListFont`; nível vem de
`OutlineElementChildLevel` (0x1C03) e `ListNodes` (0x1C26).

**Aceitação**: `checkboxes_and_unicode.one` (tem listas e caixas) sai com `1.` `2.` e indentação
aninhada; teste unitário do formatador de número romano/alfa se implementado.

### M4 — Notebook inteiro (`.onetoc2`, `.onepkg`, pastas)

**Por quê**: "converter um notebook" exige o índice; o corpus tem 20 `.onetoc2` — 3 deles já em
revision store, então o parsing de TOC é testável antes de M1. `.onepkg` é CAB (ausente no corpus,
presente em exports reais).

**Formato**: `jcidPersistablePropertyContainerForTOC` (0x00120001/0x00020001) com
`TOCEntryIndex_OidIndex`, `FileIdentityGuid`, `NotebookElementOrderingID`; `.onepkg` = CAB
(`[MS-CAB]`) contendo `.onetoc2` + seções.

**Passos**: ler o TOC após M1; montar a árvore (grupo → seção → arquivo `.one`); resolver nomes via
`SectionDisplayName`/`NotebookManagementEntityGuid`; escrever `index.md` por nível e uma pasta por
seção; paralelizar por seção; colisão de nomes com sufixo estável (hash curto do GUID).

**Aceitação**: "Notebook created on OneNote App" gera árvore com as 4 seções e o grupo; `index.md`
lista páginas na ordem do TOC; nenhuma página perdida (contagem == soma das seções).

### M5 — Ink, tags e metadados

**Por quê**: 198 ink containers ignorados; 5 definições de tag; nenhum metadado de página na saída.

**Passos**: ink → marcador `> [ink: N strokes, WxH]` (rasterizar exigiria renderizador, ver §6);
`NoteTagSharedDefinition`/`NoteTagLabel` → emoji/`[tag]` antes do parágrafo; front matter com
`CachedTitleString`, `CreationTimeStamp`/`LastModifiedTimeStamp`, autores
(`AuthorOriginal`/`AuthorMostRecent`), `PageLevel`.

**Aceitação**: páginas `Ink Missing…` ganham o marcador de ink; uma página com tag mostra o rótulo;
front matter validado como YAML por parser externo no teste.

### M6 — Arquivos "sujos" e criptografia

**Criptografia (feito)**: `RevisionManifestStart6FND.odcsDefault == 0x0002` diz que *todos* os
property sets daquela revisão estão cifrados e "MUST be ignored". O parser agora lê `odcsDefault`
(offset 44 do payload), **descarta** os grupos daquela revisão, conta em
`ObjectSpace::encrypted_revisions` e emite aviso `N encrypted revision(s) skipped`. Coberto por dois
testes sintéticos (0x0002 e 0x0000), porque o corpus não tem nenhum caso (0 de 114 revisões).
O `odcs` por objeto só existe em `ObjectDeclarationWithRefCountBody` (nós 0x2D/0x2E, usados em
`.onetoc2`) e a spec exige zero, então não há gate adicional a fazer ali.

**Por quê**: já apareceu um caso real (texto só em declaração superada, resolvido por variantes);
arquivos com escrita pendente podem esconder conteúdo no log.

**Passos (sujeira)**: parsear `TransactionLogFragment`/`TransactionEntry` (contagens por `FileNodeListID`),
honrar o corte em `cTransactionsInLog`, e ler `HashedChunkList` (`fcsHashedChunkList`, nós 0x0C2).
Hoje as contagens do log já foram conferidas contra o parse e batem; falta usá-las como limite.

**Aceitação**: `test-tika-3970-dupetext.one` continua com "Sunday morning" exatamente 1 vez;
nenhuma duplicação introduzida; warning quando um arquivo tem transações pendentes.

### M7 — Robustez, desempenho e relatório

**Passos**: `--report <json>` com contagens, avisos e cobertura por tipo de objeto; limites de
profundidade/tamanho contra bombas; fuzz com `cargo fuzz` sobre os 3 fixtures corrompidos; medir
throughput no arquivo de 2,6 MB e no de 879 KB; opção `--strict` que falha se houver qualquer objeto
não renderizado (gate de CI futuro).

**Aceitação**: p95 < 1 s por MB; nenhum pânico em 10 mil mutações; `--strict` passa em todo arquivo
íntegro do corpus.

## 4. Verificação

1. **Corpus com lock**: `tests/corpus.lock` (URL + SHA-256 de cada arquivo) e `scripts/fetch-corpus.sh`;
   binários grandes fora do git.
2. **Oráculos independentes**: asserções do `OneNoteParserTest` (Tika) para 2016/chinês/dupetext/2
   páginas; amostras do `onenote.rs` como cobertura estrutural.
3. **Invariantes automáticos** em todo arquivo: padding, dangling, `cbExpectedFileLength`, zero
   pânico, determinismo (duas execuções ⇒ mesmo SHA-256 da saída).
4. **Snapshot de contagens**: total de páginas/assets/objetos por tipo, para detectar regressão
   silenciosa em qualquer refactor.

## 5. Sequência

| Ordem | Entrega | Cobertura esperada do corpus |
| --- | --- | --- |
| hoje | revision store plano | 14/44 `.one` |
| M1 | desembrulho FSSHTTPB | ~40/44 `.one` + 20 `.onetoc2` |
| M2 + M3 | formatação e listas | qualidade de saída |
| M4 | notebook (TOC/onepkg) | árvore completa |
| M5 | ink/tags/metadados | completude de conteúdo |
| M6 + M7 | sujeira e robustez | confiabilidade |

M2 e M3 são independentes de M1 e podem correr em paralelo com ele (arquivos diferentes); M4 depende
de M1.

## 6. Decisões abertas

1. **Dependências**: zero-dependency hoje. Deflate (se existir no payload) e CAB pedem inflate. Opções:
   (a) `miniz_oxide` como única dependência; (b) implementar inflate/CAB à mão (~600 linhas, risco de
   bug sutil). Recomendação: (a), com feature flag.
2. **Cor/highlight em Markdown**: `<span style="color:#RRGGBB">` (fiel, HTML) vs nota de rodapé vs
   ignorar. Recomendação: `<span>`, com `--no-inline-html` para desligar.
3. **Ink**: marcador textual (recomendado) vs rasterizar (precisa de renderizador de traços, fora do
   escopo de um conversor de texto).
4. **Version history / páginas de conflito**: hoje excluídas. Manter excluídas (padrão) ou
   `--include-history`.
5. **Criptografia**: conteúdo cifrado é omitido + reportado (recomendado). Decifrar exigiria a
   chave (`ObjectDataEncryptionKeyV2FND`) e o algoritmo, que a especificação pública não define —
   alternativa: expor `--fail-on-encrypted` para quem preferir erro a saída parcial.
6. **Nomes de arquivo**: derivar do título da página (legível) ou do GUID (estável). Recomendação:
   título + sufixo do GUID curto, com modo `--stable-names`.

## 7. Não-objetivos

Escrever `.one`, falar com a API Graph/OneDrive, sincronizar, editar, renderizar layout (posições
absolutas, tamanhos de página) e version history por padrão.

## 8. Reproduzir a medição

```sh
# corpus
gh api "repos/msiemens/onenote.rs/git/trees/HEAD?recursive=1" \
  --jq '.tree[] | select(.path|test("\\.(one|onetoc2)$")) | "\(.size)\t\(.path)"'

# contagens por tipo de objeto
for f in files/*.one; do target/release/onenote2md inspect "$f"; done
```

## 9. Entrega em Python (bindings)

O conversor é consumido como biblioteca Python por meio de um módulo PyO3 em `python/`, que depende
deste crate — não há reimplementação. A lógica de escrita saiu do binário para `src/output.rs`
(`Section::read/markdown/images/materialize/write`) e `src/notebook.rs` (`Notebook::read`,
`write_tree`), e o CLI passou a delegar nelas, então as duas frentes produzem bytes idênticos.

API: `onenote.read_section(path|bytes)`, `onenote.read_notebook(toc)`,
`onenote.read_package(onepkg)`; objetos `Section`/`Page`/`Image`/`Notebook`/`Package`, com
`markdown(front_matter=)`, `write(out, assets, front_matter=)`, `all_sections()` e `errors`.
Verificação: `python -m pytest python/tests -q` (10 testes) mais a bateria Rust; os pacotes de teste
são CABs gerados em Python, ou seja, um oráculo independente do leitor.

Nomes de imagem vêm de uma única passada (`Section::named_images`), usada por `markdown()`,
`images()`, `materialize()` e pelas páginas no Python: um link sempre nomeia um arquivo que o
chamador recebe. Referências sem payload (clipping web, cache externo) não são inventadas — vão para
`missing_images()`, e o invariante testado é `links ⊆ images() ∪ missing_images()`. Como o mesmo GUID
aparece várias vezes no arquivo, a passada deduplica: o corpus escreve 62 arquivos para 71
ocorrências de link (antes eram cópias `-2`, `-3` do mesmo byte).

