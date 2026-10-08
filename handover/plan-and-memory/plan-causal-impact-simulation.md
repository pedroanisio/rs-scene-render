---
disclaimer:
  notice: >-
    Nenhuma informação deste documento deve ser tomada como garantida.
    Estimativas de prazo, metas de desempenho e itens marcados como
    "a verificar" são julgamento do autor, não medições.
  generated_by: "Claude Fable 5.1 via Claude Code"
  date: "2026-10-03"
---

# Plano: motor de simulação causal de impacto (100/100)

Base: commit `60a458c` (branch `cinematic-impact-checkpoint`). Este plano complementa
[`srep-0000-cinematic-impact.md`](srep-0000-cinematic-impact.md) e o ledger
[`tools/evidence/cinematic-impact.json`](tools/evidence/cinematic-impact.json); não os substitui.

## 1. Objetivo

O objetivo não é encenar uma cena. É um motor que **executa a simulação e seus efeitos**.

**Definição de 100/100.** Dada uma cena que declara apenas:

- um corpo com massa, tamanho, velocidade inicial e ângulo;
- um ambiente (oceano com profundidade, fundo, atmosfera);
- as ligações entre eles (quem colide com quem, quem alimenta quem),

o motor produz sozinho a cratera, a resposta da água, os ejetos, a pluma e as interações
entre eles, **sem nenhum evento de efeito com `time` escrito à mão**, em resolução que
sustente um close em 3840×2160, mantendo as garantias atuais (determinismo, replay
reverso, limites explícitos de memória e trabalho, erros em vez de fallback silencioso).

**Fora do escopo.** Previsão científica em nível de hidrocódigo (iSALE, equação de estado
de rochas, validação contra dados de Chicxulub). O alvo é causal e fisicamente fundamentado,
verificado contra casos canônicos, não preditivo. Ver decisão D1.

**Restrição herdada do SREP.** Nenhum nó específico de Chicxulub: todo mecanismo novo
tem de ser genérico (contato, colisor, fonte), reutilizável em outras cenas.

## 2. Estado atual

### 2.1 Medições feitas em 2026-10-03

Cena de teste `examples/cinematic-impact/hero.scene.xml` (mesma simulação de
`impact.scene.xml`, com luz de céu neutra para os defeitos ficarem visíveis), 1280×720,
8 amostras, pluma 64×52×64, binário de `60a458c`, RTX 6000 Ada:

| Tempo da cena | Tempo de parede | Observação |
|---|---|---|
| 0,8 s | 19 s | Antes do impacto |
| 1,5 s | 30 s | Pluma é uma esfera lisa laranja |
| 3,0 s | 79 s | Pluma é uma bola cinza lisa; retângulo escuro sob ela |

O que os quadros mostram:

- **Pluma:** bola sem turbulência, sem coluna nem cogumelo.
- **Água:** plana; a onda do `waterImpulse` não é perceptível; ruído de sal e pimenta
  mesmo com `denoise="true"`.
- **Ejetos:** pontos de tamanho uniforme espalhados em todas as direções; não há cortina.
- **Retângulo escuro sob a pluma em t=3 s:** investigado por Mercurio (commit `33248a1`)
  e **não é defeito**. É o reflexo da pluma na água, cortado pela borda do quadro; uma
  câmera espelhada no plano d'água reproduz a mancha no mesmo lugar. O item 0.4 fica
  encerrado sem correção, com dois testes novos de volume visto por reflexão.
- **Custo:** a diferença entre 0,8 s e 3,0 s é quase toda simulação de fumaça em CPU.

### 2.2 Fatos do código que determinam o plano

| Fato | Onde | Consequência |
|---|---|---|
| O fundo do oceano (`bed_y`) é passado uma vez em `Ocean::new` | `crates/sr-sim/src/ocean.rs` | A cratera não move a água |
| Cada solver tem relógio, replay e checkpoints próprios (`Ocean::at`, `pyro::Timeline::at`, `Emitter::at`) e exige entradas que dependam só do tempo e da cena imutável | `crates/sr-sim/src/` | Acoplamento de mão dupla não cabe na arquitetura atual |
| `Simulation::step` clona o estado inteiro a cada passo; a projeção é gradiente conjugado com pré-condicionador de Jacobi e vizinhança calculada por célula | `crates/sr-sim/src/pyro.rs` | Custo cresce mais que linearmente com a resolução |
| `sr-sim` não usa `rayon` (já é dependência do workspace) | `crates/sr-sim/Cargo.toml` | Solvers rodam em um núcleo |
| A pressão calculada na projeção é descartada | `pyro.rs`, `project` | Não há como a fumaça empurrar corpos |
| O motor rígido percorre pares de contato só para contabilizar memória | `crates/sr-sim/src/physics3d.rs` | Impulsos de colisão não chegam a outros sistemas |
| Volumes usam espalhamento único com transmitância por marcha de raio | `crates/sr-gpu/src/volume.wgsl` | Plumas densas ficam chapadas |
| Denoiser À-trous de 5 passes guiado por normal e albedo do primeiro impacto | `crates/sr-gpu/src/pathtrace.rs` | Não remove ruído de reflexão especular na água |
| Oceano é águas rasas de primeira ordem (Lax–Friedrichs local); borda aberta não é absorvente | SREP, "Implemented numerical contract" | Ondas difundem e refletem na borda |
| Cratera é deformação autoral; fratura é corte geométrico sorteado; `expansion` da fumaça é fonte de divergência autoral | SREP | Nenhum deles responde à energia do impacto |

### 2.3 Rubrica da nota

A nota passa a ser a soma de cinco dimensões, para que "100" seja verificável:

| Dimensão | Peso | Hoje | F0 | F1 | F2 | F3 | F4 | F5 |
|---|---|---|---|---|---|---|---|---|
| Causalidade (efeitos derivam da simulação) | 25 | 3 | 3 | 3 | 21 | 25 | 25 | 25 |
| Física modelada | 25 | 5 | 5 | 8 | 9 | 24 | 24 | 25 |
| Resolução e desempenho | 15 | 3 | 3 | 12 | 12 | 13 | 15 | 15 |
| Render | 20 | 5 | 6 | 7 | 7 | 8 | 19 | 20 |
| Validação e entrega | 15 | 2 | 4 | 5 | 6 | 8 | 10 | 15 |
| **Total** | **100** | **18** | **21** | **35** | **55** | **78** | **93** | **100** |

Os pontos por fase são estimativa; cada fase só "paga" quando seu critério de aceite passa.

### 2.4 Estado do roadmap em 2026-10-06 (reconciliação contra o SREP, o ledger e o histórico)

| Fase | Itens | Concluídos | Parciais | Abertos | Observação |
|---|---|---|---|---|---|
| 0 | 5 | 5 | 0 | 0 | tudo por nós |
| 1 | 18 | 14 | 0 | 4 | 12 por nós, 2 já existiam na base (1.13, 1.15); abertos 1.5, 1.9, 1.11 (estacionado), 1.14 |
| 2 | 12 | 9 | 1 | 2 | parcial 2.2 (`ocean@density` ausente); abertos rasto e pressão da fumaça, adiados |
| 3 | 7 | 0 | 1 | 6 | parcial 3.3 (só crescimento da cratera); fratura por contato já feita, contada na Fase 2 |
| 4 | 7 | 1 | 1 | 5 | 4.3 luz pela água feita; 4.4 absorção feita, espuma como albedo não |
| 5 | 5 | 0 | 3 | 2 | sequência UHD completa e `goal_complete` em aberto |
| §6 | 9 | 5 | 1 | 4 | 3 por nós, 2 partilhados com a base |
| §7 | 8 | 4 | 1 | 3 | nomes reais: `@crater`, `@source`, `heatFraction`, `energyFraction` |
| **Total** | **70** | **36** | **8** | **26** | 34 por nós, 2 anteriores à base, 0 por outros desenvolvedores |

Fora do roadmap, por nós: buraco negro de Schwarzschild, amostrador de normais visíveis,
cavidade de entrada na água, resposta do fundo por profundidade, captura pela cratera,
arestas internas, aviso W02, checkpoints do oceano e da espuma, log de quadros rígidos,
crédito de pressão por dono, anel de estados do oceano, 36 correções da revisão de código.

## 3. Princípios que valem para todas as fases

1. **Determinismo.** Mesmo resultado bit a bit com qualquer número de threads e em replay
   reverso. Reduções paralelas (somas, produtos internos) usam blocos de tamanho fixo
   somados em ordem fixa.
2. **CPU é a referência.** Backend de GPU para solvers só como opção posterior, comparado
   contra a CPU (D2).
3. **Limites explícitos.** Todo solver novo declara orçamento de memória, de checkpoints e
   de trabalho por busca; estourar é erro.
4. **Compatibilidade.** Cenas 1.1/1.2/1.3 existentes produzem os mesmos resultados. Todo
   comportamento novo que altere números entra por atributo com padrão igual ao atual.
5. **Evidência.** Teste vermelho antes, verde depois, registrado no ledger, como já é feito.
6. **Esquema.** Cada atributo novo exige XSD, Schematron, regra Rust, fixtures do corpus e
   seção no SREP. O esquema canônico muda só via SREP aceito em sr-core.

## 4. Fases

### Fase 0 — Linha de base e defeitos visíveis (2–3 dias) → 21

Objetivo: ter um antes/depois comparável para todas as fases seguintes.

| # | Tarefa | Onde |
|---|---|---|
| 0.1 | Tempo por solver (rígido, oceano, fumaça, partículas) e por etapa de render nas estatísticas do `render --stats`, se ainda não existir (a verificar) | `crates/scene-render`, `crates/sr-eval` |
| 0.2 | Script de sonda: N quadros a 720p pela fila `sr-gpu`, com tempos e RSS, saída em JSON para o ledger | `tools/` |
| 0.3 | Cena de carga com luz neutra (já criada: `hero.scene.xml`, `sky.hdr`, `tools/make_sky_hdr.py`) | `examples/cinematic-impact/` |
| 0.4 | Investigar e corrigir o retângulo escuro sob a pluma em t=3 s. Hipótese: limite do domínio do volume entrando no cálculo de sombra ou de intervalo (`volume_interval`, `volume_transmittance`) | `crates/sr-gpu/src/volume.wgsl` |
| 0.5 | Reproduzir o ruído da água com denoise ligado em um teste mínimo (plano especular sob céu) | `crates/sr-gpu/tests/` |

Critério de aceite: sequência completa de 144 quadros a 720p renderizada em modo estrito,
com relatório de tempo por etapa no ledger; teste de regressão para o item 0.4.

### Fase 1 — Desempenho para ter resolução (1–2 semanas) → 35

Objetivo: fumaça em 192³ ou mais, oceano sem difusão visível, centenas de milhares de
partículas, tudo dentro de tempos utilizáveis.

**Fumaça (`crates/sr-sim/src/pyro.rs`)**

| # | Tarefa | Nota |
|---|---|---|
| 1.1 | Perfil em 64³, 128³ e 192³; registrar tempo por etapa (obstáculos, advecção, projeção) | Define a ordem das tarefas seguintes |
| 1.2 | Paralelizar laços por célula (advecção, forças, aplicação do operador, atualizações de vetor) com `rayon`. Cada célula é independente, então o resultado é idêntico ao serial | Manter as reduções seriais nesta etapa: zero mudança numérica |
| 1.3 | Operador de pressão sem `neighbours()`/`Option` por célula: máscara de faces abertas pré-calculada e índices lineares | Mesma aritmética, mesma ordem |
| 1.4 | Eliminar o clone do estado por passo: dois buffers e troca só em caso de sucesso (preserva a atomicidade em erro) | Reduz o orçamento de ~512 bytes por célula |
| 1.5 | Voxelização de obstáculos por caixa envolvente em vez de testar todas as células contra todos os colisores | |
| 1.6 | Pré-condicionador multigrid para o gradiente conjugado (MGPCG). Iterações deixam de crescer com a resolução | Muda os bits do resultado: entra com atributo `pyro@solver="jacobi\|multigrid"`, padrão `jacobi` |
| 1.7 | Advecção MacCormack com limitador, para preservar detalhe na mesma resolução | Atributo `pyro@advection`, padrão atual |
| 1.8 | Reduções paralelas em blocos de ordem fixa | Muda bits: junto com 1.6, sob o mesmo atributo |
| 1.9 | (Opcional, após medir) região ativa: resolver só a caixa onde há densidade, calor ou velocidade | Só se 1.2–1.8 não bastarem |

Medição da tarefa 1.1 (commit `5a080f3`, código serial, cena do impacto, média por passo):

| Grade | Total | Advecção | Forças | Projeção | Iterações do gradiente conjugado |
|---|---|---|---|---|---|
| 64×52×64 | 0,59 s | 0,24 s | 0,10 s | 0,22 s | 65 em média, 135 no impulso |
| 128×104×128 | 8,6 s | 2,0 s | 0,8 s | 5,7 s | 190 em média, até 325 |
| 192×156×192 | 36,9 s | 6,7 s | 2,7 s | 26,8 s | 256 em média, até 417 |

Consequências:

- A projeção domina em alta resolução porque as iterações crescem com a grade. Com
  `pressureIterations="200"` a cena falha em 128³ no passo do impulso. O multigrid (1.6)
  vira a tarefa principal da onda 2.
- As reduções seriais impõem um piso de cerca de 0,5 s por passo em 128³, então a onda 1
  (bit a bit idêntica) não alcança 1 s por passo.
- A voxelização de obstáculos custa 9–31 ms por passo: a tarefa 1.5 saiu da onda 1.

Resultado da onda 1 (commits até `477fdee`, bit a bit idêntica ao commit base) e do
multigrid da onda 2 (commit `8d3ca51`, `Spec::solver`, padrão Jacobi), medidos por
Saturno com 8 threads e a máquina carregada pelos outros agentes:

| Grade | Base, serial | Onda 1, Jacobi | Onda 2, multigrid | Iterações Jacobi → multigrid |
|---|---|---|---|---|
| 128³, regime | 6,1 s | 1,4 s | 1,0–1,4 s | 107 → 7 |
| 128³, impulso | 11,5 s | 2,5 s | 1,2–1,6 s | 275 → 14 |
| 192³, regime | 28 s | 7,0 s | 3,5–4,3 s | 179 → 9 |
| 192³, impulso | 54 s | 13 s | 3,9–5,9 s | 417 → 16 |

Numa janela com a máquina mais livre ele mediu 0,6–0,75 s em 128³ e 1,9–2,3 s em 192³
com multigrid. A certificação das metas fica para uma rodada com a máquina quieta, no
fechamento da fase. Com multigrid a advecção passa a ser a maior parcela do passo.

Demais entregas da onda 2 (Saturno, ainda a verificar na integração):

- `dfd30a5`: estimativa de memória de 512 para 320 bytes por célula (pior caso por
  construção ~270; pico medido em 192³: 741 MB com Jacobi, 894 MB com multigrid).
- `ffadd69`: amostragem e traçado mais rápidos sem mudar bits (advecção −15% em 128³).
- `3941afa`: advecção MacCormack com limitador (`Spec::advection`, padrão
  semi-lagrangiano). No teste de vórtice livre mantém 96,6% da energia cinética e 95,7%
  da enstrofia após 60 passos, contra 68,8% e 61,6% do semi-lagrangiano, sem a energia
  crescer. Custa 2,4 a 2,5 vezes a advecção atual. A cena do impacto roda 144 passos em
  64³ e 128³ com multigrid + MacCormack sem erro.
- Tarefa nova **1.18 — custo da fumaça no caminho do avaliador**. Medição de Saturno, por
  quadro em regime, multigrid + MacCormack, 8 threads, máquina carregada (valores
  absolutos inflados; as proporções valem mais):

  | Parcela | 128³ | 192³ |
  |---|---|---|
  | Passo do solver | 1,5–1,9 s | 5,7–6,9 s |
  | Exportação para volume esparso | 0,6–1,2 s | 2,2–4,2 s |
  | Chave de cache | 0,06–0,16 s | 0,25–0,45 s |
  | Checkpoint (1 a cada 24 passos) | 0,01 s | 0,38 s |
  | Preparo para a GPU na CPU | < 1 ms | ~1 ms |

  - Exportação + chave são 35–45% do quadro. Mais da metade da exportação são os três
    canais de velocidade, que são densos e gravados voxel a voxel.
  - Cada amostra fracionária (motion blur) refaz a exportação inteira mesmo sem passo novo.
  - Com os orçamentos padrão (256 MiB) nenhuma das duas resoluções roda: 128³ falha no
    orçamento do passo e 192³ no de checkpoints. São erros claros, não lentidão. Quando
    só o estado inicial cabe, a busca reversa refaz todos os passos desde o início.
  - Aprovado: exportação sem gravar valores de fundo e por tijolo inteiro, sem reexportar
    no mesmo passo, chave de cache em blocos paralelos, e velocidades sólidas esparsas
    (estado de ~89 para ~41 bytes por célula, para o checkpoint de 192³ caber no padrão).
    Meta: custo fora do solver em 192³ de ~3,3 s para até 0,5 s por quadro.
  - Os padrões de memória ficam como estão; o SREP ganha uma tabela de orçamento mínimo
    por resolução.

Cena de carga em resolução-alvo (`examples/cinematic-impact/hero-hires.scene.xml`,
commits `285b3fc` e `b4e59b5`, por Netuno): fumaça 128×104×128 com multigrid e
MacCormack, oceano 720×720 de segunda ordem, 100 mil partículas. Avaliação sem GPU, com
a máquina carregada: avanço de 0 a 6 s em cerca de 290 s e busca a frio direta a 6 s em
279 s, com pico de 2,3 GiB de RSS. O que a cena expôs:

- O teto do esquema para `ocean@maxWork` (1 bilhão) não permitia renderizar um quadro
  isolado em t ≥ 1,5 s nessa grade. Teto elevado para 1 trilhão, padrão mantido. O mesmo
  vale para `whitewater@maxWork` (em andamento).
- A malha de superfície do oceano a 720×720 células precisa de 151 MiB, acima do padrão
  de 128; a cena declara `surfaceMemoryMiB="256"`.
- A espuma não tem checkpoint: toda busca para trás recomeça do zero. Candidato para a
  próxima fase.

Metas revisadas, em 8 núcleos. Onda 1: 128³ em até 1,5 s por passo em regime e 3 s no
impulso; 192³ em até 5 s em regime. Onda 2: 128³ em até 1 s por passo e 192³ em até 4 s.

**Oceano (`crates/sr-sim/src/ocean.rs`, `ocean/flux.rs`)**

| # | Tarefa | Nota |
|---|---|---|
| 1.10 | Reconstrução de segunda ordem (MUSCL com limitador minmod) e integração SSP-RK2 | Atributo `ocean@order="1\|2"`, padrão 1 |
| 1.11 | Borda absorvente por camada de esponja | **Estacionada** (branch `parked/ocean-sponge`, commit `b727f6b`); ver medição abaixo |
| 1.12 | Paralelismo por linha, mesmo critério de determinismo | |

Medição das tarefas 1.10 e 1.11 (commits `b46ad86` e `b727f6b`, por Netuno):

| Células por comprimento de onda | Ordem 1 | Ordem 2, minmod | Ordem 2, van Leer | Ordem 2, MC |
|---|---|---|---|---|
| 10 | 99,96% | 97% | 88% | 77% |
| 20 | 98% | 64% | 31% | 18% |
| 40 | 85% | 22% | 4,8% | 2,6% |
| 80 | 62% | 4,5% | 0,6% | 0,3% |

Perda de amplitude após 5 comprimentos de onda, borda periódica. A ordem 2 custa cerca de
3 vezes mais por subpasso e tem metade do limite de CFL.

- O limitador MC (central monotonizado) passou em todos os testes e é o limitador único
  da ordem 2 (commit `b5b05a2`). O fluxo continua sendo o Lax–Friedrichs local.
- Paralelismo por bandas de linhas (commit `5a4ed6f`, bit a bit idêntico ao serial),
  tempo por passo de 1/24 s medido por Netuno com a máquina carregada: 1 milhão de
  células na ordem 2 cai de 1,85 s (1 thread) para 0,37 s (8 threads); 4 milhões, de
  7,7 s para 2,3 s. A ordem 2 passa a cobrar 400 bytes por célula no orçamento residente
  (commit `bf75e21`); a ordem 1 mantém 256.
- A ordem 2 precisa de pelo menos 20 células por comprimento de onda. Na cena de impacto
  atual o pulso tem 8 a 16; a cena de aceitação deve usar célula de oceano menor, o que é
  barato (o limite é de 4 milhões de células).
- A premissa da tarefa 1.11 estava errada: a borda `open` reflete só 3–5% de um pulso
  suave em 2D (zero em 1D), e a esponja não ganha nessa métrica (4–6%). O ganho real da
  esponja é o resíduo tardio (0,13% contra 1,6% em t=120 s) e o nível de volta ao repouso,
  pequeno demais para justificar atributos novos agora. O código fica estacionado como
  candidato a zona de relaxação para o acoplamento da Fase 3.

**Partículas e render**

| # | Tarefa | Nota |
|---|---|---|
| 1.13 | Preparação de desenho sem um `Draw3` por partícula | Já está na lista do outro agente; coordenar (D3) |
| 1.14 | Estrutura de aceleração persistente entre quadros no path tracer | Idem |
| 1.15 | Verificar e, se preciso, elevar o teto de `maxParticles` para 1 milhão com orçamento de memória | A verificar o limite atual do esquema |

Medição da sonda de render (commits `7e6fcb3` e `ca85e5e`, cena de carga a 1280×720,
8 amostras, 4 mil partículas):

| Tempo da cena | Parede | Fumaça (CPU) | Traçado (GPU) | Preparo de desenho + BVH (CPU) |
|---|---|---|---|---|
| 0,8 s | 9,0 s | 7,6 s | 0,05 s | 0,16 s |
| 1,5 s | 26,2 s | 16,2 s | 8,3 s | 0,20 s |
| 3,0 s | 74,9 s | 35,2 s | 37,0 s | 0,20 s |

Consequências:

- O preparo por partícula e a BVH custam 0,2 s por quadro com 4 mil partículas. As
  tarefas 1.13 e 1.14 ficam suspensas até a medição com 100 mil e 500 mil partículas.
- O traçado sobe de 0,05 s para 37 s quando a pluma densa entra: é a marcha de volume com
  sombra aninhada. Entra a tarefa nova **1.16 — custo da marcha de volume** (análise e
  proposta primeiro), que passa à frente de 1.13 e 1.14.
- Medição de escala (Mercurio, t=1,5 s, 1280×720): com 100 mil partículas o preparo em
  CPU do render soma 0,54 s por quadro; com 500 mil, 3,85 s (9% da parede), dos quais
  0,23 s no preparo de desenho e 1,1 s na BVH. O traçado não cresce com as partículas.
  **As tarefas 1.13 e 1.14 ficam encerradas nesta fase, sem implementação.** Candidata
  para a Fase 4: montar a cena do path tracer sem um material por partícula (2,0 s dos
  3,85 s em 500 mil).
- Tarefa 1.15 respondida: 500 mil partículas renderizam com `maxMemoryMiB="2048"` e pico
  de 2,4 GiB de RSS. Com o padrão de 256 MiB o solver recusa, as partículas somem do
  quadro e o render termina com sucesso, só com uma nota (erro apenas com `--strict`).
  Entra a tarefa **1.17 — consistência do relato de falhas de simulação** (levantamento e
  proposta primeiro).
- Análises de Mercurio (2026-10-04), todas aprovadas para implementação nesta fase:
  - **Ruído na água (item 0.5).** Causa medida: a probabilidade de amostrar o lóbulo
    especular é fixa em 0,25 para dielétricos. Na água, 10% dos pixels ficam escuros a
    8 amostras (0,75⁸) e o erro relativo é 0,59; o denoiser não mistura pixel claro com
    escuro. Correção: probabilidade pela razão das reflectâncias, sem viés. Erro previsto
    por cálculo: 0,08.
  - **Marcha de volume (1.16).** Com a pluma de t=3 s congelada: 29,2 s de traçado, dos
    quais ~99% é a sombra marchada dentro do volume (0,33 s sem ela). Entram duas
    otimizações exatas, sem mudar pixels: salto de espaço vazio e busca do bloco uma vez
    por amostra (estimativa 4 a 10 vezes, não medida). A grade de transmitância por luz,
    que muda pixels e exige atributo, vai para a Fase 4 junto com o espalhamento múltiplo.
    Mesmo com as duas otimizações, UHD fica em estimados 30 a 60 s por quadro.
  - **Relato de falhas (1.17).** Orçamento estourado dá erro em fumaça, oceano e cratera,
    mas só uma nota em partículas e fratura (o objeto some do quadro), e fumaça e oceano
    reportam uma mensagem que não é a causa. Passa a ser erro em todos, com a causa real.
- Resultados de Mercurio (2026-10-04):
  - **Ruído na água corrigido** (commit `815018e`): probabilidade do lóbulo especular pela
    razão das reflectâncias. Na água da cena, a 8 amostras, o erro relativo caiu de 0,593
    para 0,119 e os pixels escuros de 9,2% para 0%; a expectativa de água, plástico e
    metal a 4096 amostras mudou no máximo 0,03%. Nenhum valor esperado existente mudou.
  - **Marcha de volume: as otimizações exatas não rendem.** Buscar o bloco uma vez por
    amostra ficou mais lento (descartada). O salto de células exatamente vazias ganha só
    1,05 vez, porque a densidade exportada tem uma cauda não nula em quase toda a grade
    (406 de 448 blocos existem; valores espalhados até 1e-34). Pular células com
    densidade ≤ 1e-12 dá 1,6 a 2,1 vezes com o PNG idêntico. Decisão: aprovar o salto com
    limiar definido como limite de profundidade óptica ignorada (≤ 1e-10 em qualquer
    raio), só na marcha de sombra, e testar um diretório denso de blocos no lugar da
    busca binária (exato). Cortar a cauda na exportação muda dados, chaves e bakes: fica
    para a Fase 4, junto com a grade de transmitância.
  - **Marcha de volume, resultado** (commits `2afdecd` e `eb45dd7`): salto de células
    desprezíveis na marcha de sombra mais diretório denso de blocos. Traçado da pluma
    congelada de t=3 s a 1280×720: 26,8 s → 15,3 s (salto) → 4,7 s (salto + diretório),
    5,75 vezes, com o PNG idêntico ao original nas quatro variantes medidas. Extrapolação
    não medida para UHD: cerca de 42 s por quadro.
- Problema conhecido do ambiente: no llvmpipe deste host (LLVM 15; os goldens foram
  gerados com LLVM 19) marchas longas de volume escurecem em quartos exatos. Dois testes
  já falham por isso no commit base: `pathtrace_instances` e
  `volume.rs::advected_cache_documents_move_fields_replay_and_validate_channels`. Ambos
  passam na NVIDIA. Não tratado nesta fase; testes novos de volume usam marchas curtas.
  Um terceiro teste é instável no mesmo adaptador:
  `raster::tests::solid_fill_specialization_matches_general_rasterization_exactly` falhou
  em 1 de 4 execuções isoladas. O rasterizador não mudou desde o commit base, então é
  tratado como instabilidade do ambiente, sem investigação.

**Medição com a máquina quieta** (2026-10-04, `phase1/integration` em `02a99d0`, 8
núcleos, RTX 6000 Ada, uma execução de cada medida):

Fumaça, tempo por passo com 8 threads (impulso / regime):

| Grade | Jacobi + semi-lagrangiano | Multigrid + semi-lagrangiano | Multigrid + MacCormack | Meta |
|---|---|---|---|---|
| 64×52×64 | 0,23 s / 0,13 s | 0,09 s / 0,08 s | 0,14 s / 0,13 s | — |
| 128×104×128 | 1,98 s / 0,97 s | 0,54 s / 0,49 s | 0,89 s / 0,78 s | onda 1: 3 s / 1,5 s; onda 2: 1 s |
| 192×156×192 | 10,3 s / 5,0 s | 1,96 s / 1,63 s | 3,06 s / 2,65 s | onda 1: 5 s em regime; onda 2: 4 s |

Todas as metas da fumaça foram atingidas; a de 5 s em regime com Jacobi em 192³ ficou no
limite (5,001 s). Contra o commit base em série (medido sob carga): 128³ em regime de
6,1 s para 0,49 s; 192³ de 28 s para 1,63 s. Exportação do volume: 0,03 s em 128³ e
0,11 s em 192³.

Cena em resolução-alvo no avaliador, sem GPU: 0 a 6 s de simulação em 151 s; teste
inteiro em 167 s; pico de 1,74 GiB.

Render, um quadro a 8 amostras (parede / fumaça / traçado na GPU):

| Cena e tamanho | t = 3 s | Commit base |
|---|---|---|
| Carga, 1280×720 | 20,5 s / 7,6 s / 10,2 s | 74,9 s / 35,2 s / 37,0 s (sob carga) |
| Carga, 3840×2160 | 132 s / 7,5 s / 121 s | não medido |
| Resolução-alvo, 1280×720 | 103 s / 55,8 s / 31,7 s | não rodava |
| Resolução-alvo, 3840×2160 | 427 s / 54,5 s / 355 s | não rodava |

**O orçamento provisório de 60 s por quadro UHD não foi atingido.** O traçado em UHD
leva 121 s na cena de carga e 355 s na de resolução-alvo; a extrapolação de 42 s estava
errada. O custo continua sendo a sombra marchada dentro do volume. A grade de
transmitância por luz, prevista para a Fase 4, passa a ser pré-requisito para qualquer
sequência UHD e deve ser a primeira tarefa de render da próxima fase.

Critério de aceite:

- Teste que roda a mesma simulação com 1, 2 e 8 threads e compara os bytes do estado.
- Testes existentes de `sr-sim` e `sr-eval` passam sem alterar valores esperados (padrões).
- Tempos por passo medidos nas três resoluções e registrados no ledger.
- Onda de segunda ordem: erro de amplitude após propagar N comprimentos de onda menor que
  o de primeira ordem por fator definido no teste.

**Fechamento da Fase 1 (2026-10-04).** Código integrado e verificado em
`phase1/integration` (`8af44e9`), branch local, sem push e sem merge na linha principal.

Critérios de aceite:

| Critério | Situação |
|---|---|
| Mesma simulação com 1, 2 e 8 threads, bytes iguais | Atendido (fumaça e oceano) |
| Testes existentes passam sem alterar valores esperados | Atendido (sr-sim, sr-volume, sr-eval, sr-model, scene-render e sr-deliver; sr-gpu nas suítes de path tracer, na NVIDIA) |
| Tempos por passo medidos nas três resoluções e registrados no ledger | Atendido, uma execução com a máquina quieta |
| Onda de segunda ordem com erro menor que a de primeira por fator declarado | Atendido (5,5 a 181 vezes, conforme a resolução) |
| Sequência de 144 quadros a 720p em modo estrito (linha de base) | Atendido: 18,9 min, 7,9 s por quadro em média, pico de 653 MiB |

Nota pela rubrica da seção 2.3: cerca de 33, contra 35 previstos. A diferença está em
resolução e desempenho: os solvers atingem as metas, mas o render em UHD não cabe no
orçamento.

Em aberto, por ordem de importância para as próximas fases:

1. Traçado em UHD: 121 a 355 s por quadro. Exige a grade de transmitância por luz.
2. A pluma toma a forma da caixa do domínio no fim da sequência: o domínio da fumaça é
   fixo e corta a pluma nas bordas. A região ativa (tarefa 1.9) ou um domínio que
   acompanha a pluma resolve; não foi feito.
3. Cauda de densidade não nula em quase toda a grade exportada.
4. Espuma sem checkpoint: toda busca para trás recomeça do zero.
5. Oceano de ordem 2 a 1 milhão de células: 0,50 a 0,53 s por passo, contra a meta de 0,5 s.
6. Falhas no adaptador de software deste host (três no sr-gpu, uma delas instável, e
   oito no sr-deliver), todas ausentes na NVIDIA.
7. Não rodados: o workspace inteiro de uma vez, o teste `backends`, MSRV e outras
   plataformas.

### Fase 2 — Acoplamento causal (2–3 semanas) → 55

Objetivo: os efeitos passam a ser consequência do corpo e do ambiente.

**Início da Fase 2 (2026-10-04).** Branch de integração `phase2/integration`, criado em
`8af44e9`. Distribuição inicial, no mesmo modelo de trabalho da Fase 1 (análise ou
projeto primeiro, implementação depois do OK do coordenador):

| Agente | Branch | Primeira entrega | Depois |
|---|---|---|---|
| Saturno | `phase2/cosim` | Projeto do agendador de co-simulação (2.0), só documento | Registro determinístico de contatos do motor rígido (base de 2.3) |
| Netuno | `phase2/ocean-bed` | Projeto do oceano com fundo móvel e colisores (2.1) | Solver com fundo por passo, ligação com a cratera, colisores, esquema |
| Mercurio | `phase2/volume-light` | Protótipo e proposta da grade de transmitância por luz | Implementação; meta de 45 s de traçado por quadro UHD na cena de resolução-alvo |

Decisões de partida:

- O que é de mão única (o consumidor amostra o produtor pelo tempo, como a fumaça já faz
  com colisores) é feito já, sem esperar o agendador: oceano ← cratera e corpos, e o
  registro de contatos. O que forma ciclo (corpos ← água, contato → cratera → colisor do
  terreno) espera o projeto do agendador.
- A grade de transmitância, que estava na Fase 4, foi antecipada: sem ela nenhuma
  sequência UHD é viável, e ela não depende do acoplamento.
- O domínio fixo da fumaça que corta a pluma fica na fila do Saturno, depois do projeto
  do agendador.

**Projeto do agendador aprovado (Saturno, 2026-10-04), que substitui o esboço abaixo:**

- Levantamento do que existe: ordem fixa em `Runtime::apply` (rígido, emissores 2D,
  partículas 3D, oceano, agentes, fumaça); cada solver tem relógio e checkpoints
  próprios; nenhum solver lê a saída de outro; nada observa contatos; o oceano não
  recebe o grafo nem a física. O cache de física em disco é validado só por contagens,
  então um cache obsoleto é usado em silêncio.
- Mão única, sem agendador: contato → partículas, contato → fumaça, cratera → oceano,
  fumaça → partículas, partículas → oceano pelo nível de repouso. O laço contato →
  cratera → colisor do terreno fecha dentro do próprio mundo rígido, com atraso natural
  de um passo.
- Ciclos de verdade: oceano ↔ corpos, fumaça ↔ corpos, partículas ↔ oceano (altura
  local), partículas ↔ fumaça (rastro).
- Solução para os ciclos: grupo acoplado implícito pelas ligações; passo macro igual ao
  menor passo dos membros, com os demais múltiplos inteiros; troca só por registros
  pequenos (contato, parâmetros de cratera, carga por corpo, impulso de água), nunca por
  campos de grade; leitura estritamente do passado, com atraso de um passo do consumidor.
- **Replay por membro com registro imutável**, no lugar do checkpoint conjunto: cada
  membro mantém os próprios checkpoints e pode ser reexecutado sozinho lendo o registro.
  Evita duplicar o estado da fumaça (67 MiB por checkpoint em 128³) e repetir a fumaça ao
  rebobinar o oceano.
- Cache de física passa a `SRPHYS04`, com contatos, velocidades e resumo SHA-256 do
  documento; divergência é erro. Versões 1 a 3 continuam lidas como hoje.
- Sem elemento nem atributo novo para o grupo: passo derivado, orçamento do registro como
  constante interna com erro ao estourar.
- Ordem: registro de contatos; registro no avaliador e cache v4; cratera por contato;
  partículas por contato; fumaça por contato; esqueleto do grupo {rígido, oceano};
  corpos ← água; demais pares; endurecimento.
- Riscos anotados: instabilidade do acoplamento explícito corpo ↔ água (massa
  adicionada); atraso de um passo da fumaça (até 41 ms); espuma sem checkpoint (vai para
  Netuno antes do grupo); consumidores empurrando o mundo rígido para trás (a medir).

Andamento (2026-10-04):

- Registro de contatos do mundo rígido integrado (`779c08e`): instante, corpos, ponto,
  normal, impulso e velocidade relativa por passo, com limiar de impulso na origem e
  limites explícitos. Custo: +30–40% no passo rígido com todos os corpos em contato.
- **Defeito do motor atual, achado por medição:** o mundo rígido restaura o checkpoint de
  até 1 s atrás sempre que um consumidor pede um instante anterior ao passo atual, mesmo
  por um passo. As partículas 3D pedem instantes até um quadro antes, e o motion blur
  também. Na cena de resolução-alvo com o impactor como corpo rígido: 6 ms por quadro
  viram 215–650 ms sem blur e 5–16 s com 16 amostras. Correção aprovada: log de quadros
  do mundo rígido, idêntico bit a bit, com orçamento em bytes.
- Lei de escala da cratera aprovada: Holsapple 1993 (gravidade + resistência), constantes
  por material de uma única fonte, entrada pela velocidade normal do contato. Atributos:
  `crater@source`, `crater@targetMaterial`, opcionais `targetDensity`, `strength`,
  `gravity`. Cortes da primeira versão: sem alongamento em impacto oblíquo, uma cratera
  por elemento (primeiro contato acima do limiar), sem efeito da lâmina d'água sobre o
  alvo, sem modelo de ricochete.

- Log de quadros do mundo rígido (`a9c9f1b`): com 16 amostras de blur o tempo no mundo
  rígido caiu de 89,9 s para 0,50 s em 40 quadros, sem nenhuma restauração; resposta
  idêntica bit a bit à de um mundo sem log. O mundo 2D tem a mesma condição no código,
  não medida pelo avaliador: candidato.
- Ejetos e fumaça por cratera, proposta aprovada:
  - Ejetos: lei de Housen & Holsapple 2011 (velocidade por posição de lançamento), massa
    total 0,8·ρ·V, ângulo de 45° ± 15°, material mais rápido primeiro; viés de massa por
    azimute em impacto oblíquo ligado aos limiares publicados de 45° e 25°. Cortada a
    escala de velocidade a jusante, por falta de suporte.
  - Fumaça: não há fração de energia publicada, então são parâmetros do motor com padrão
    documentado. Densidade = fração de volume de sólidos, com `dustFraction` (0,01) do
    volume ejetado; calor = `heatFraction` (0,1) da energia cinética, com sen^1,5 do
    ângulo; temperatura pela massa de poeira e `specificHeat`; teto `maxTemperature`
    (5000 K); expansão derivada do aquecimento por gás ideal, sem valor autoral.
  - Consequência assumida: um impacto lento em unidades físicas quase não aquece. A cena
    de aceitação terá de usar velocidade física de impacto para ter bola de fogo.
  - Atributos: `crater@id`, `burst@crater`, `pyroSource@crater`, `pyroImpulse@crater`.

- Oceano com fundo móvel e corpos (Netuno, commits `f1ab36e` a `877ca36`), com
  `ocean@colliders`. Medições dele:
  - Onda gerada por fundo que sobe chega com erro de posição de 0,6% (ordem 1) e 1,9%
    (ordem 2) contra √(g·h)·t; volume conservado a 1e-14.
  - Caso da cena (cratera de profundidade 25 sob lâmina de 12): sem profundidade
    negativa nas duas ordens; o anel em volta drena a ~10% da lâmina e não seca.
  - Cratera sem `waterImpulse` gera onda, e crateras maiores geram ondas maiores (3,4,
    5,3 e 11,3 para profundidades 4, 8 e 16).
  - Custo de amostrar o fundo a 720×720: +0,015 s por passo, sob carga.
  - **Limite encontrado:** pela ocupação do corpo, a altura da onda satura com a
    velocidade de entrada (0,637, 0,661, 0,660 para ×0,5, ×1, ×2), porque o volume
    deslocado é o mesmo. A dependência com a energia vem pela cratera e pelo momento
    horizontal. Impacto em água funda, sem tocar o fundo, não produz onda dependente da
    energia nesta fase; a cavidade transiente na água pela lei de escala entra como
    tarefa do Netuno depois da cratera por contato.
- Fila do Netuno: checkpoint da espuma; testes das lacunas que ele declarou (malha como
  fundo e como corpo, oceano animado, outras orientações, relógio não invertível).

- Registro no avaliador e cache de física `SRPHYS04` (Saturno, `dc53f20` e `2e6db26`):
  corrigido um defeito do próprio registro (par em repouso era registrado para sempre);
  cache com velocidades, contatos e resumo do documento, com divergência virando erro.
- Checkpoints da espuma e do solver do oceano (Netuno, `e00ab60` e `90e0ac2`): a busca
  para trás a 1,5 s no oceano da cena de resolução-alvo caiu de 8,6 s para 2,0 s sob
  carga. O solver do oceano só guardava os últimos 5 alvos visitados, então toda busca
  para trás recomeçava do início; agora guarda um por segundo, com afinamento.
- Lacunas do oceano fechadas com teste (`2f6a127`): malha como fundo e como corpo, oceano
  animado, outras orientações do plano, relógio não invertível.
- **Defeito encontrado, depois corrigido na análise:** a hipótese inicial era que ids de
  colisores dentro de instância de símbolo não eram encontrados em nenhum solver. Netuno
  conferiu antes de mexer: fumaça e partículas já resolvem no escopo léxico, porque o
  compilador reescreve o atributo; só o nó do oceano faltava nessa lista. Correção
  mínima no compilador, com os três testes como regressão.
- **Cratera por contato pronta** (Saturno, `5c7e37e` a `9a1b6c9`): o mundo rígido detecta
  o primeiro impacto do corpo de origem sobre o dono da cratera; a lei de Holsapple
  (`sr_sim::cratering`, função pura) dá raio, profundidade, borda e duração; render,
  partículas, fumaça e oceano leem a mesma cratera. Atributos `crater@source`,
  `targetMaterial`, `targetDensity`, `strength`, `gravity`, com regras CRT6 a CRT8.
  Teste de ponta a ponta sem nenhum tempo na cena: corpo a 60, 100 e 150 m/s dá cratera
  de profundidade 8,7, 10,6 e 12,3 e onda de 9,8, 11,6 e 13,6, com volume de água
  conservado e replay idêntico. Pendente: resolver `crater@source` pelo compilador (hoje
  é busca literal) e exercitar a cratera por fonte no render em GPU.
- Redistribuição: os ejetos por cratera passam do Saturno para o Netuno (lei dos ejetos
  como função pura e emissão por eventos no solver de partículas). Saturno segue com a
  cratera por contato, o grupo acoplado e o empuxo.
- Atributo novo aprovado: `whitewater@checkpointMemoryMiB` (padrão 64).

- **Grade de transmitância por luz: protótipo medido e proposta aprovada** (Mercurio).
  Traçado em UHD, t=3 s, 8 amostras, pluma congelada: cena de resolução-alvo completa de
  352,7 s para 8,8 s; cena de carga sem ejetos de 53,0 s para 1,86 s. Construção da
  grade: 0,02 a 0,8 s por quadro; memória calculada de 4,5 a 35 MiB por pluma. Erro a
  512 amostras contra referência de 2048: 57,8 dB e ΔE máximo 1,46, com piso de ruído de
  59,1 dB e 1,54. Fase anisotrópica exige direções fixas (o domo pré-integrado dá
  31 dB). Achado do protótipo: a visibilidade de superfícies por passo contra 100 mil
  instâncias custava 100 s; assada na grade, some. Atributos: `medium@lighting`
  (`exact` por padrão, ou `grid`), `lightGridCell`, `lightGridDomeDirections`,
  `lightGridMemoryMiB`. O caminho exato fica intocado byte a byte, em módulo de shader
  próprio. A medir na implementação: custo com amostras de motion blur, luz de área
  perto do volume, ejetos menores que uma célula como oclusores, e a cena ao vivo.
- Motion blur por componente (Saturno, 40 quadros, 16 amostras): motor rígido 0,10 s,
  fumaça 11,7 s (não muda com blur), oceano 10,0 s (antes dos checkpoints do Netuno),
  partículas 3D 31,7 s. **As partículas 3D reconstroem o estado a cada pedido para
  trás**: defeito de custo, na fila do Netuno.
- Grupo acoplado {rígido, oceano}: desenho de implementação aprovado. Oceano avança
  antes do rígido; a carga volta por registro imutável; alcance do grupo = instante
  pedido mais o maior adiantamento que qualquer consumidor do mundo rígido pede; carga
  ainda não calculada é erro explícito.

- **Esqueleto do grupo acoplado pronto** (Saturno, `af9823e` e `f3cdbae`): registro de
  troca imutável (`sr_sim::exchange::ExchangeLog`), carga por passo e corpo no mundo
  rígido (`Driver3::load`), grupo implícito {rígido, oceano}. Grupo sem carga é idêntico
  bit a bit a sem grupo. Dois achados: o atraso tem de ser de um passo inteiro do oceano
  (a versão com atraso curto era não causal, e o próprio erro de divergência acusou); e
  um erro era engolido (quando o mundo rígido falhava, o oceano via um mundo vazio).
  Também corrigido: o torque aplicado num passo não era zerado no seguinte.
- Empuxo e arrasto, projeto aprovado: empuxo instantâneo no passo rígido, pelo volume
  submerso contra o plano de superfície do último passo do oceano; reação horizontal
  pelo momento que o corpo deu à água, sem coeficiente; arrasto de forma quadrático na
  vertical (`ocean@bodyDrag`, padrão 1,0), acrescentado por mim porque só com empuxo o
  corpo oscila para sempre; erro quando ω·dt ≥ 1,8. Atributo `ocean@bodyCoupling`
  (`none` por padrão, `buoyancy`, `full`).
- **Ordem revista:** a fumaça por cratera vem antes do empuxo, por pesar mais na cena de
  aceitação. Saturno: `crater@id`, fumaça por cratera, depois o lado rígido do empuxo.
  Netuno: conserto do oceano em símbolos, lei dos ejetos e emissão por eventos, ligação
  dos ejetos no avaliador, defeito de custo das partículas com blur, amostragem do
  oceano por corpo, cavidade na água.

- **Fumaça por cratera pronta** (Saturno, `c67ffba`, `9c7af61`, `968c277`; `crater@id`
  em `b917f74`): poeira e calor derivados da cratera e do impacto
  (`sr_sim::cratering::smoke`), expansão derivada do aquecimento dentro do solver, fonte
  no ponto de impacto com a duração da cratera. Atributos `pyroSource@crater` e
  `pyroImpulse@crater` com `heatFraction`, `dustFraction`, `specificHeat`,
  `maxTemperature`; regras PYC1 a PYC4. Achado: a poeira não era conservada na grade (a
  esfera cobria centros de célula sem relação com o seu volume); corrigido.
  **Limite registrado:** com os padrões o aquecimento é de poucas centenas de kelvin
  mesmo a 20 km/s, porque o modelo aquece toda a poeira e ela cresce com a cratera
  (ΔT ~ U^0,35). Não há bola de fogo incandescente nesta fase; ela vem da massa fundida e
  vaporizada e da onda de choque, que são da Fase 3. Os padrões não foram ajustados para
  compensar. Como a densidade passa a ser fração de volume de sólidos (~1e-3 por célula),
  a cena tem de declarar `medium@extinction` na proporção.

- **Empuxo, lado rígido, pronto** (Saturno, `972b7a6` a `b9a8dbd`): volume submerso e
  centroide por recorte contra o plano da superfície (`sr_sim::hydrostatics`), empuxo
  pelo centroide, arrasto de forma na vertical, erro de instabilidade, erro para cache de
  física com grupo acoplado. `ocean@bodyCoupling` (`none` por padrão, `buoyancy`) e
  `ocean@bodyDrag`; regras OCN8 e OCN9. Uma bola de densidade 500 assenta a menos de 3 cm
  do calado previsto. Achados: carga que depende da posição tem de ser avaliada no ponto
  médio do passo (no início ela adiciona 8% de energia em 2 s); o arrasto quadrático
  amortece devagar (3 cm só depois de 40 a 80 s). Decisão: não acrescentar amortecimento
  linear; ele vem por física quando o corpo ler a superfície que ele mesmo perturba
  (amostragem do oceano por corpo, na fila do Netuno). Limites: o corpo cavalga o nível
  de repouso, não a onda; rolamento não amortecido; sem massa adicionada; `full` (reação
  horizontal) espera o momento por corpo.
- **Domínio da fumaça: a minha leitura do defeito estava errada.** Medição do Saturno: em
  6 s a pluma só toca a face de cima no fim (5,5 a 6,0 s), perde menos de 2% de massa
  pelas bordas e nunca chega às laterais. O que altera a pluma é a face aberta perto da
  fonte (4 a 7 células do fundo), que muda o escoamento desde ~2 s: num domínio duas
  vezes mais alto a cauda fica 12 a 23 células mais baixa e o pico de densidade da cena
  de carga cai de 0,671 para 0,525. É em boa parte posicionamento do domínio na cena.
  A janela que segue a pluma (`pyro@follow`) fica estacionada como candidata para planos
  longos: custaria ~1,8 s por passo e 0,67 GiB na cena de resolução-alvo, contra 4,1 s e
  1,9 GiB de um domínio fixo grande. Pendente: isolar qual face causa o efeito e propor
  um aviso de validação para fonte perto de face aberta.
- **Cenas e testes de aceitação da fase** atribuídos ao Saturno: impacto em terra (corpo,
  cratera por contato, ejetos, fumaça) e impacto no oceano (corpo, cratera, oceano com
  colliders e empuxo, sem `waterImpulse`), em unidades físicas, sem nenhum atributo de
  tempo em efeitos; testes de monotonia (velocidade, massa, ângulo), conservação e
  replay. Limite assumido: a cena do oceano não tem fumaça por cratera, porque a cratera
  está debaixo d'água e o motor não tem fumaça dentro de água.
- **Diagnóstico do domínio da fumaça fechado** (Saturno): a causa é a face aberta de
  baixo. Afastar só a de cima quase não muda nada (pico −8%); afastar só a de baixo sobe o
  pico em 31% e alonga a cauda. O pico fica dentro de 2% do valor de face distante a
  partir de 13 células entre a borda da fonte e a face. Aviso de validação aprovado:
  fonte ou impulso ativo a menos de 12 células de uma face aberta (dado de uma cena e uma
  face; laterais não testadas).
- **Cenas e testes de aceitação entregues** (`3887da1`, `7a5e32a`):
  `impact-land.scene.xml` e `impact-ocean.scene.xml`, em metros, rocha de raio 2 m e
  90 t a 100 m/s e 60°, sem nenhum atributo de tempo em efeitos. 13 testes passam:
  nada antes do impacto; cratera, poeira e calor crescem com velocidade, massa e ângulo;
  a onda só da cratera cresce com os três; volume de água exato; replay idêntico em
  qualquer ordem e após descarte de checkpoints. Custo: 10,5 s (terra) e 3,6 s (oceano)
  para 25 quadros. Quatro casos deixados como testes ignorados, com números:
  1. Com a rocha como colisor do oceano, a onda não é monótona com a velocidade (7,6,
     4,7, 6,5 m): sem arrasto horizontal a rocha cruza a água a Froude 2 a 5 e domina a
     onda de proa. **Critério de aceite ainda não atendido para a cena completa**; espera
     a reação horizontal (`full`), que depende da amostragem do oceano por corpo. Essa
     amostragem subiu na fila do Netuno.
  2. Temperatura da poeira plana com o ângulo (19 K): é o que a lei dá; a expectativa
     foi retirada. Calor e poeira crescem. A 100 m/s a poeira aquece 19 K.
  3. Eixo e velocidade normal da cratera saíam da normal de contato, inclinada e
     dependente da malha (+5%). Correção aprovada: usar a normal geométrica da superfície
     no ponto de impacto.
  4. Com colisor de chão, a fumaça perdia 45% da poeira. Correção aprovada: dividir pelas
     células livres.
- **Ejetos por cratera prontos** (Netuno, `d520296`, `aa904a2`, `d52a4c5`): lei dos ejetos
  em `sr_sim::cratering::ejecta`, nascimento por eventos no solver de partículas
  (`Driver::births`, `Particle.mass`), `burst@crater` com `angle` e `angleSpread`, regras
  P3D7 a P3D10. Massa total dos ejetos = 0,8 da cratera (±20% no teste). Limites: a lei
  não foi conferida contra o artigo pelo Netuno; dono móvel não carrega os lançamentos.
- **Amostragem do oceano por corpo pronta** (Netuno, `d7d7a45`): plano da superfície,
  velocidade média da água e fundo sob a pegada de cada corpo, e o momento horizontal
  que cada corpo deu à água; +12 bytes por célula nos oceanos com corpos. Destrava a
  reação horizontal (`bodyCoupling="full"`), agora com o Saturno.
- Verificação da integração em `00ed0b3` (empuxo, cenas de aceitação, ejetos): sr-sim
  282, sr-eval 273, sr-model 91, sr-3d 82, sr-gpu na NVIDIA 75, CLI 61; corpus
  regenerado idêntico ao versionado (277 documentos); lint, formatação e higiene limpos.
- Teste do CLI que o Netuno viu falhar (`watch_notices_an_included_documents_asset`):
  não reproduzido (suíte inteira e seis execuções isoladas passam).
- Custo das partículas 3D com motion blur corrigido (Netuno, `e9a935e`, `5c1a11b`): 16
  amostras de 25,0 s para 9,3 s em 40 quadros com 20 mil partículas, hash idêntico. A
  causa era o pedido para trás restaurar o checkpoint de até 1 s, não a reconstrução do
  estado. Sobra um passo parcial por instante de amostra.
- Correções dos casos das cenas de aceitação (Saturno): eixo da cratera pela normal
  geométrica da superfície (86,63 contra 86,60 m/s; mesma resposta com 8 a 160
  segmentos); poeira espalhada pelas células livres (5,5% da lei com colisor de chão);
  ejetos na cena de terra com massa = 0,8·ρ·V exata, alcance 10,7, 18,0 e 26,3 m para
  60, 100 e 150 m/s, e deslocamento a jusante com o ângulo.
- **Cavidade na água pela entrada do corpo** (Netuno, `c740c0d`, ainda não integrado):
  `<waterImpulse source="corpo"/>`, opt-in; evento = primeira vez que o corpo cruza o
  nível de repouso; raio e volume pela lei de escala em alvo água; limitada a 90% da água
  do disco central, nunca erro em lâmina rasa. A onda só da cavidade cresce com a
  velocidade (0,0033, 0,0071, 0,0170) e com a massa. **Combinada com a ocupação do corpo
  não é monótona** (0,0326, 0,0390, 0,0276): cavidade e ocupação têm sinais opostos perto
  da entrada. Segundo dado apontando para a reação horizontal como o que falta.
  Pendência antes de integrar: os ids OCN8 a OCN10 colidem com os do `bodyCoupling`.
- **Defeitos em aberto:** (1) ejetos com o chão em cratera como colisor param o solver de
  partículas com "mais de 16 colisões num passo" (Netuno, diagnóstico primeiro); (2)
  fumaça com o chão móvel como colisor fica 5 vezes mais lenta, por voxelizar o chão a
  cada passo (candidato a otimização; a cena fica sem esse colisor).
- **Grade de luz implementada** (Mercurio, `fb705be` e `2993015`): módulo de shader e
  bind group separados, construídos só quando um meio pede; atributos `medium@lighting`,
  `lightGridCell`, `lightGridDomeDirections`, `lightGridMemoryMiB`, regra VOL10. O
  caminho exato ficou com o sha256 idêntico nas quatro variantes da pluma congelada.
  Cena de resolução-alvo ao vivo em UHD, t=3 s: 8,76 s de traçado e 1,46 s de construção
  da grade. Erro contra o exato a 512 amostras: 56,4 dB e ΔE máximo 1,50 sem ejetos.
  **Limites documentados:** com 100 mil ejetos menores que uma célula fazendo sombra, o
  erro passa do limite declarado (47,8 dB e ΔE máximo 3,89, mesmo com 8 amostras de
  visibilidade por nó); luz de área próxima do volume não tem penumbra (rect: 65, 53 e
  43 dB a 60, 14 e 9 unidades). Motion blur refaz a grade a cada subquadro (~14% do
  traçado em UHD).
- Verificação da integração em `f21d465`: sr-sim 295, sr-eval 281, sr-model 91, sr-3d 82,
  sr-gpu na NVIDIA 75, CLI 61; corpus idêntico; nenhum arquivo em merge nem marcador.
- Verificação da grade de luz em `89249df` (os hashes `fb705be` e `2993015` eram de antes
  do rebase): sr-model 92; corpus idêntico (283 documentos); sr-gpu na NVIDIA 164 testes
  em 15 binários, com os 11 da grade; lint, formatação e higiene limpos. Medição minha na
  cena de resolução-alvo ao vivo, UHD, t=3 s, modo grade: 8,76 s de traçado e 1,46 s de
  construção da grade, iguais aos do Mercurio; parede 175 s, dos quais 128 s de fumaça a
  frio com a máquina carregada; pico de 2,7 GiB.
- **Cena de carga a 720p, t=3 s, no caminho padrão: imagem idêntica byte a byte à do fim
  da Fase 1** (`953a7b38…`), com tudo da Fase 2 integrado. Nenhuma cena existente mudou.
- **Reação horizontal e leitura da superfície prontas** (Saturno, `fc75160`, `cffa632`):
  `bodyCoupling="full"`; o corpo flutua sobre o plano da superfície sob a sua pegada e
  recebe de volta o momento que deu à água. Resultados dele:
  - Com a rocha como colisor do oceano, a onda agora cresce com a velocidade (1,29, 1,54,
    2,17 m para 60, 100, 150 m/s; era 7,6, 4,7, 6,5) e com a massa (1,23, 1,54, 2,77 m).
    **O critério de aceite que estava aberto fecha para velocidade e massa.**
  - Com o ângulo, decresce (2,41, 1,54, 0,09 m): a onda é a de proa, que cresce com a
    velocidade horizontal. A exigência de crescer com o ângulo foi excesso meu e foi
    retirada; a onda só da cratera continua crescendo com o ângulo. O mergulho vertical
    dar 9 cm é o caso que a cavidade na água cobre; ela entra na cena e as varreduras
    são medidas de novo.
  - Uma bola a 3 m/s percorre 6,0 m em 2 s sem a reação e 0,77 m com ela.
  - Momento de corpo + água: erro de 13%, 10% e 4,8% para passos de 0,1, 0,05 e 0,025 s
    (cai com o passo; parte é o atraso de um passo). De 2 a 4% do momento que a água
    ganha não é creditado a nenhum corpo: candidato para o Netuno.
  - Amortecimento de radiação apareceu sem coeficiente: a bola assenta a 3 cm do calado
    em 8,75 s lendo o plano, contra 24 s no nível de repouso; sem arrasto ela assenta em
    vez de oscilar para sempre.
  - Defeito achado e corrigido: a janela de passos rígidos de um passo canônico variava
    (11, 12 ou 13), ±8% do momento do passo.
- Arestas internas de malhas no motor rígido (medição do Saturno): com a opção de
  corrigi-las, o desvio lateral da rocha cai de 7,35 m para 0,42 m em 4 s e a normal de
  contato fica exata, mas três testes mudam e toda cena com corpo deslizando sobre malha
  muda. Decisão: opt-in por atributo em `<physics>`, padrão igual a hoje, com o registro
  de contatos adaptado.
- Diagnóstico visual da pluma fechado com dois quadros: no domínio original a pluma tem
  base reta e lados verticais; com a face de baixo afastada fica arredondada; o topo é
  cortado pelo enquadramento nos dois.
- Correção dos ejetos contra o chão (Netuno, em andamento): nascimento do lado do ar,
  aprovado; e teto próprio para recuperação de penetração (colisões sem avanço de
  tempo), no lugar de mudar a folga, para não alterar os bits de cenas existentes.
- **Exercício de render das cenas de aceitação** (Mercurio, cópias locais com
  `lighting="grid"`, 1280×720): a cratera por contato deforma o terreno no raster e no
  path tracer (cova de ~10 m com borda suave); os ejetos nascem no impacto; a fumaça da
  cratera aparece; o oceano mostra a onda. O render é desprezível perto da simulação
  (terra: 17,8 s de parede em t=5,5 s, 13 s de fumaça; oceano: 4,6 s). Nenhuma falha
  silenciosa. Defeitos encontrados:
  - Grade de luz dava erro com o meio vazio antes do impacto (do próprio Mercurio):
    correção aprovada.
  - **Faixas na fumaça espessa com a grade**: com profundidade óptica por célula de ~30
    (extinção 30000, a derivada de grãos de 100 µm) a interpolação da transmitância cria
    faixas; com ~3 somem. Decisão: documentar e avisar por nota agora; depois interpolar
    profundidade óptica, porque a poeira física é espessa. Orientação de cena: extinção
    300 mal visível, 3000 lê como fumaça, 30000 vira cúpula opaca.
  - **A rocha é lançada depois do impacto em terra**: 6 m acima e à direita da cratera
    em t=2 s, com restituição 0. Para o Saturno diagnosticar.
  - **Crista pontuda de ~8 m sobre a rocha submersa**, com borda em serra: a ocupação
    levanta a coluna inteira pela espessura do corpo, o que só vale para perturbação
    mais larga que a profundidade. Para o Netuno avaliar a atenuação por profundidade
    (filtro de Kajiura) no deslocamento do fundo e da ocupação.
  - Menores, para a fase de render: pontos escuros no horizonte do plano de água
    distante; manchas no chão a 8 amostras.
- **Rocha lançada depois do impacto: diagnosticado** (Saturno). O salto de 6 a 10 m era
  das arestas internas da malha (a normal de contato inclinada dava velocidade para
  cima); some com `physics@fixInternalEdges`, agora opt-in e ligado nas duas cenas de
  aceitação. O que sobra: a rocha rola a 35 m/s (5/7 de 50 m/s, rolamento sem
  deslizamento) com 17% da energia inicial, e sai da cratera antes de ela se formar (cruza
  a borda em 1,68 s, quando a cratera está a 15%). Nenhum dos mecanismos do terreno a
  empurra, e a energia nunca cresce depois do impacto. Falta um sumidouro de energia
  para o projétil. **Aprovado `crater@capture`** (opt-in): força resistente constante
  m·U²/(2·d) contra a velocidade relativa, dentro do raio da borda, até o corpo parar;
  é modelo do motor (força média de penetração), não lei publicada.
- Verificação da integração em `d65f1f0` (cavidade na água): sr-sim 302, sr-eval 287,
  sr-model 92, sr-3d 82, sr-gpu na NVIDIA 86, CLI 61; corpus idêntico (288); nenhum id de
  regra duplicado.
- Correção dos ejetos contra o chão pronta (Netuno, `b793000`): nascimento do lado do ar e
  teto próprio de 64 para recuperação de penetração; folga intacta; digest de cena com
  colisor sobreposto igual antes e depois. A cena de terra roda de 1,6 a 5,9 s com o chão
  como colisor. Os ejetos pousam mas quase nenhum para (33 de 4000 abaixo de 0,5 m/s aos
  5,9 s): atrito padrão zero; a cena deve declarar o atrito.
- **A resposta do oceano a corpos compactos está errada por ~100 vezes** (medição do
  Netuno): esfera de 2 m de raio no fundo de 20 m de água levanta a superfície 3,00 m no
  modelo contra 0,024 m da teoria linear. A ocupação levanta a coluna inteira, o que só
  vale para perturbação mais larga que a profundidade. O momento também está inflado
  (cerca de 4 vezes o arrasto real de uma esfera). Na cena de aceitação: máxima de
  0,51 m sem a rocha e 10,62 m com ela. **Aprovado:** `ocean@bedResponse` =
  `depthFiltered` (novo padrão) ou `hydrostatic` (o de hoje, para regressão). No modo
  filtrado, o levantamento do fundo e da ocupação passa pelo núcleo de Kajiura
  (1/cosh(k·h); generalização do motor para volume acima do fundo), e a relaxação de
  momento vira arrasto de forma com `ocean@bodyDrag`. Consequências assumidas: a onda da
  cratera diminui em água funda e as varreduras das cenas de aceitação mudam de valor;
  Saturno mede de novo. Limite: corrige a resposta impulsiva; a propagação continua de
  águas rasas, não dispersiva.
- **`crater@capture` e o aviso W02 prontos** (Saturno, `3b58a0f` a `7587463`). Com o
  capture a rocha para a 0,04, 0,49 e 1,22 m do ponto de impacto (90°, 60°, 30°) e desce
  com o fundo da cova; energia cinética de 4,5e8 J no impacto para 8e3 a 1e5 J; nunca
  cresce; sem o atributo, bits idênticos. Limite: a rocha fica balançando no fundo da
  cova, por não haver resistência ao rolamento. W02 avisa fonte de fumaça a menos de 12
  células de uma face aberta; a cena de carga agora avisa (7 células).
- Varreduras das cenas de aceitação com `full` + cavidade + arestas corrigidas + capture
  (modo hidrostático de hoje; valem como prova de monotonia, não como alturas): onda no
  oceano crescente com a velocidade (4,54, 8,93, 12,11 m) e com a massa (6,80, 8,93,
  11,62 m), também no mergulho vertical; com o ângulo 4,07, 8,93, 10,20 m (registrado);
  cratera, poeira, calor, massa e alcance dos ejetos crescentes com velocidade, massa e
  ângulo em terra; ejetos deslocados a jusante no impacto oblíquo.
- Arrasto dos ejetos pelo gás (fumaça → partículas), projeto aprovado: `particles3D@gas`
  referencia o volume de fumaça nativo; o arrasto existente (`drag`) passa a ser contra a
  velocidade do gás dentro do domínio e contra ar parado fora; gás interpolado no tempo
  entre os dois passos de fumaça que cercam o instante; a fumaça é avaliada antes das
  partículas só nesses documentos. Aviso de escala do próprio Saturno: para rochas de
  0,34 m o arrasto real é desprezível (k ≈ 0,03 por segundo); o acoplamento serve para
  partículas do tamanho de poeira. O arrasto físico derivado da massa fica para a fase
  seguinte.
- Grade de luz, fechamento dos dois pontos (Mercurio): meio vazio antes do impacto não
  exige malha (`e144806`); nota em `stats.unsupported` quando a profundidade óptica por
  célula passa de 4 (`00874e2`). **Correção dele ao que tinha relatado:** as faixas na
  fumaça muito espessa não são defeito da grade; vêm dos próprios voxels de 1 m do campo
  de densidade (cada um com profundidade óptica ~40 a extinção 30000) e aparecem igual
  no caminho exato a 512 amostras (degrau médio entre linhas 0,0089 exato, 0,0091
  grade). A variante que interpola profundidade óptica foi medida e descartada: +0,4 dB
  bruto e +2 a 2,5 dB com desfoque no caso espesso, nada no fino.
- Verificação da integração em `ee8a2fb`: sr-sim 306, sr-eval 295, sr-model 92, sr-3d 82,
  sr-gpu na NVIDIA 86, CLI 61; corpus idêntico (290); imagem do caminho padrão idêntica
  à do fim da Fase 1.
- Antecipado da fase de render, só análise: luz através da água (o fundo submerso sai
  preto no path tracer, então a cratera da cena do oceano não aparece).
- Itens do plano retirados desta fase: fratura ativada por contato (vai para a Fase 3,
  junto com a fratura por tensão); pressão da fumaça sobre corpos; rastro das partículas
  na fumaça.
- Arrasto dos ejetos pelo gás pronto (Saturno, quatro commits mais testes): 1,5 µs por
  partícula por passo; replay idêntico; sem `gas`, bits iguais aos de hoje. Defeito que os
  testes dele expuseram e ele corrigiu: com leitor de gás, cada amostra de motion blur
  fazia a fumaça voltar a um checkpoint. O estado extra de fumaça que a correção guarda
  passa a ser cobrado no orçamento de memória (pendente).
- Atrito dos ejetos: com 0,7 e restituição 0,15, na cena autoral, 3513 de 4000 ficam em
  repouso aos 5,9 s (com atrito zero, 7). **Não está na cena**, porque com atrito maior
  que zero a variante da rocha de 270 t para o solver de partículas ("mais de 16 colisões
  num passo"), e com o raio de colisão físico (0,17 m) 754 partículas atravessam o chão.
  Os dois defeitos vão para o Netuno depois do `bedResponse`.
- Respingo dos ejetos na água (partículas → oceano), projeto aprovado: `ocean@splash`;
  a partícula é removida ao cruzar o nível de repouso; agregação por célula e por passo
  num único evento esparso; volume e momento horizontal conservados. Saturno faz o lado
  das partículas agora; a parte do oceano espera o `bedResponse`.
- Verificação da integração em `7396c86` (capture, W02, correção dos ejetos): sr-sim 312,
  sr-eval 303, sr-model 94, sr-3d 82, sr-gpu na NVIDIA 86, CLI 61; corpus idêntico (293).
- **Luz através da água: análise e aprovação** (Mercurio; antecipado do item 4.3). Medido
  contra força bruta em cena mínima: o sol não chega a nada debaixo de uma superfície
  transmissiva (0,0000 contra 0,182; o quadro perde 59% da luz; 14,6 dB), porque o raio
  de sombra é bloqueado pela água; o domo chega certo (57 dB). Raio de sombra refratado
  com Fresnel, 1/η² e absorção ao longo do trecho na água: 43,8 dB (37,1 dB sem
  absorção); o raio reto dá 42,0 dB mas põe a sombra 50% longe demais. Custo: +90% de
  traçado na cena mínima. Segundo defeito achado: raio que atravessa vidro e não acerta
  nada pega o fundo 2D preto em vez do domo visível (a faixa escura no horizonte).
  Aprovado como correção, sem atributo: fundo do domo atrás de vidro; absorção de
  Beer–Lambert no path tracer (`attenuationColor`/`attenuationDistance`, que só o raster
  usava); raio de sombra refratado. Muda a imagem de cenas com material transmissivo
  (exemplo: `impact.scene.xml` +37% de brilho médio); cenas sem ele ficam idênticas.
  Limites: câmera debaixo d'água e ondas íngremes não cobertos.
- **Resposta do oceano por profundidade entregue** (Netuno, `76618aa`, `4308a71`,
  `daef17c`): `ocean@bedResponse` = `depthFiltered` (padrão) ou `hydrostatic`. Esfera de
  2 m no fundo de 20 m: 0,02367 m no centro contra 0,02427 m por quadratura (2,5%); no
  motor, 3,00 m no modo antigo e 0,026 m no filtrado. Arrasto de forma: 104,05 contra
  104,72 teórico no primeiro passo (0,6%), onde a relaxação dava ~4 vezes. Cena do
  oceano em t=2,5 s, sem cavidade: crista de 10,62 m para 0,16 m. Cratera de 6 m em 40 m
  de água: onda 6,7 vezes menor; cratera de 40 m em 6 m: 5% menor. Custo dentro do ruído
  da máquina. Limite: para o corpo móvel não há número teórico de comparação.
  **Achado:** com a cavidade ligada, a máxima continua ~9,5 m, agora por causa do anel da
  cavidade, que aparece inteiro num instante com poucas células de largura. Aprovado:
  formar a cavidade ao longo da duração da lei (~1 s nesta cena) e dar largura mínima e
  perfil suave ao anel.
- Lado das partículas do respingo na água pronto (Saturno): partícula removida ao cruzar
  o nível de repouso; volume no registro igual a 0,8·V da lei a 1e-9; `ocean@splash` e
  regra OCN13 no esquema. O oceano ainda não aplica nada (espera `Forcing::splash`, do
  Netuno); a fase não fecha nesse meio-termo.
- Verificação da integração em `285b3aa` (gás, atrito, grade): sr-sim 315, sr-eval 313,
  sr-model 94, sr-3d 82, sr-gpu na NVIDIA 156, CLI 61; corpus idêntico (295); imagem do
  caminho padrão idêntica à do fim da Fase 1.
- **Respingo dos ejetos na água completo** (Saturno, `613e87f`..`343bf23`, sobre o
  `Forcing::splash` do Netuno em `3d3df6c`): bacia fechada mantém o volume a 1e-9; o
  momento horizontal no oceano é o do registro a 1e-6; esparso e denso idênticos ao bit;
  na cena da praia, 121 de 4000 ejetos caem na água (2,44 m³). Defeito achado e
  corrigido: instantes fora da grade do oceano falhavam, porque o oceano pede o respingo
  do passo seguinte; o emissor agora é calculado até o fim do passo do oceano.
- **Varreduras de aceitação refeitas nos dois modos** (Saturno; `tools/impact_sweeps.sh`
  repete): todas as ordens afirmadas continuam valendo no modo filtrado e no
  hidrostático. Maior onda com velocidade: 4,43, 9,06, 11,70 m (filtrado) e 4,54, 8,93,
  12,11 m (hidrostático); com massa: 6,74, 9,06, 11,65 m. Onda só da cratera, sem
  cavidade: 0,0057, 0,0124, 0,0210 m no filtrado contra 0,164, 0,275, 0,400 m, ~22 vezes
  menor em 20 m de água, ordem preservada. As alturas de 9 a 12 m vêm do anel da
  cavidade instantânea e vão mudar com a cavidade ao longo do tempo.
- Integração em `f98f5a1`: correção do oceano e evento de respingo do Netuno, em
  verificação.
- **Cavidade ao longo do tempo de formação pronta** (Netuno, `187deaf`): parcelas por
  passo canônico com a duração da lei, anel de pelo menos 4 células. Na cena do mar o
  anel de 9,49 m em t=2,5 s cai para 0,86 m. **Quebra dois testes de aceitação do mar**
  (maior onda com a massa: 6,41, 8,03, 7,30 m; mergulho vertical com a velocidade: 5,43,
  8,69, 7,81 m): para as rochas maiores a profundidade da cavidade pela lei passa da
  lâmina de 20 m, o volume é limitado pela água e a crista satura. Decisão: integrar, e
  trocar a métrica do critério: onda de campo distante crescente com velocidade e massa
  fora da saturação; regime saturado só registrado, com a explicação. Saturno refaz os
  dois testes. Por um intervalo a integração terá esses dois testes falhando, anotado.
- Verificação da integração em `3e415ea` (respingo completo, varreduras nos dois modos,
  cavidade ao longo do tempo): sr-sim 338 e 1 falha, sr-eval 345 e 4 falhas, sr-model 94,
  sr-3d 82, sr-gpu na NVIDIA 88, CLI 61; corpus idêntico (300); hygiene limpo; sem
  marcadores de conflito. As 4 falhas do sr-eval são os dois testes do mar combinados,
  cada um nos módulos filtrado e hidrostático. A falha do sr-sim é nova e determinística:
  `event_sort_workspace_is_included_in_the_resident_budget` (`tests/ocean.rs:292`)
  porque `CavityPart { share, before }` levou o `Impulse` de 64 para 80 bytes e a
  pré-condição do teste (2000 impulsos < 160 000 bytes) deixou de valer. Causa em
  `187deaf`; o relatório do Netuno não incluiu `cargo test -p sr-sim`. Pedido: corrigir
  o teste derivando o orçamento do tamanho real do `Impulse`, e os relatórios passarem a
  incluir o sr-sim. `cargo fmt --check` falha em `sr-eval/src/lib.rs` (Saturno corrige).
- 2026-10-05: as três sessões dos agentes reiniciaram e perderam os aliases; estado nos
  worktrees: Saturno `343bf23` (1 arquivo não commitado, testes do mar em andamento),
  Netuno `187deaf` (7 arquivos não commitados), Mercurio `feab62b` (1 arquivo). As
  execuções em segundo plano dos agentes morreram no reinício; o que não foi relatado
  com resultado conta como não verificado.
- Estimativa de completude da Fase 2 em 2026-10-05 (rubrica minha, pesos entre
  parênteses): 2.0 grupo acoplado 100% (15); 2.1 oceano ← fundo e corpos 90% (15, falta o
  crédito de pressão por dono); 2.2 corpos ← água 100% (10); 2.3 contato como fonte 100%
  (20, fratura foi para a Fase 3); 2.4 partículas 75% (10, dois defeitos do solver de
  partículas abertos); aceitação 70% (10, dois testes do mar em reescrita, teste combinado
  por executar); render 60% (10, luz pela água implementada e não integrada, diferença
  UHD de 394 px em bisseção); fechamento 10% (10). Total ≈ 80%, contra ≈ 70% na
  estimativa anterior.
- Integração em `59e85f1` (fmt do lib.rs, Saturno) e `6a54d65` (teste do orçamento de
  ordenação derivado do tamanho do `Impulse`, Netuno `d462c56`, agora nos dois sentidos:
  recusa sem espaço para a ordenação e aceita com ele). sr-sim ocean passa; fmt e hygiene
  limpos. Verificação completa adiada para o próximo lote.
- **Correção de explicação minha** (medição de Saturno, 2026-10-05): a profundidade da
  cavidade pela lei (`cratering::crater`, água) fica entre 7,5 e 15,8 m em toda a faixa de
  aceitação, abaixo da lâmina de 20 m; a saturação da crista junto da cavidade não é
  "profundidade maior que a lâmina". Em campo distante (anel 20–40 m) a onda cresce com
  velocidade e massa na faixa inteira, nos dois modos (filtrado por massa 1,22/1,32/2,22 m;
  por velocidade 0,95/1,32/1,85 m). Netuno mede o limite real (hipótese: volume da lei
  contra `CAVITY_SHARE` da água do disco ponderado); até lá o SREP só descreve o fato.
- Diagnóstico parcial das partículas (Netuno): (a) com atrito, o limite de 16 colisões
  acaba numa perseguição de Zeno com o chão da cratera subindo a 3,9 m/s (cada colisão
  avança ~1 ms, velocidade relativa quase zero); (b) com raio 0,17 m, 678 de 4000 nasciam
  sob o chão porque o ponto de contato do mundo rígido fica ~0,1 m dentro da superfície;
  nascer do ponto mais próximo da superfície do dono (`surface::nearest`, protótipo) deixa
  109, por investigar.
- **Luz pela água, bisseção fechada** (Mercurio): a diferença de 394 px (máx. 28) na cena
  hero em UHD vem do raio de sombra refratado (`feab62b`), porque a espuma (`whitewater`)
  é transmissiva por padrão (transmission 0,6); sem a linha do whitewater o quadro UHD é
  idêntico ao bit entre binários, e os sete quadros sem vidro continuam idênticos a 720p.
  O código da água foi separado em `pathtrace_water.wgsl`, emendado só em cenas com
  material transmissivo, com teste que fixa o texto do shader plano. **Decisão:** aceito
  como correção do caminho de luz, pelo mesmo critério do lobo especular da Fase 1: a
  superfície transmissiva antes dava sombra opaca ao que está abaixo; a referência da
  verificação (hero 720p) não muda; o SREP e as notas registram os 394 px.
- **Leitura da oval retirada** (Mercurio, a pedido meu): com fundo seco a cratera do leito
  mede ~215×175 px, centro (808, 358); a oval com água tem ~1075 px de largura e centro
  deslocado 160 px: é a cavidade da superfície, e a cratera do leito não se distingue
  com água por cima. Achado adjacente: o plano `farSea` da cena do oceano é um material
  transmissivo de 40 000 m a y=0,4 e, com o traçador novo, tinge e esconde tudo abaixo
  dele. Fica para a Fase 4: a superfície distante e o oceano têm de ser uma coisa só ou o
  plano tem de ser excluído sob a pegada do oceano. Nada mudou nas cenas.
- `phase2/volume-light` rebaseado em `6a54d65`: `f112f63`, `beb543e`, `c6195de`; sweep
  completo na NVIDIA em curso; integração depois das contagens.
- **Limite da cavidade medido** (Netuno, `b78e452`, substitui a minha explicação): o kernel
  central `(1-4(r/a)²)²` é mais pontiagudo que a bacia da lei e pede pico de remoção
  `12V/(πa²)`, ~1,5× a profundidade da lei (até 24 m), contra 0,9 da água do disco (18 m
  no centro). Pedido/levado por massa: 266,5/266,5, 367,5/326,4, 862,3/567,6; parcelas
  limitadas 0 de 24, 17 de 25, 28 de 29. **Decisão:** trocar o kernel por um perfil com a
  forma da bacia da lei (pico ≈ profundidade da lei), volume conservado, limite de 0,9 só
  como proteção; critério: razão levado/pedido 1,00 nas três massas em 20 m e crista junto
  da cavidade crescendo de novo. Depois das partículas (a). Testes do mar afirmam ordem,
  não valores.
- **Ejetos nascem na superfície que a cratera tem na hora** (Netuno, `58bfc03`): ponto
  mais próximo da superfície do dono (`sr_sim::surface::nearest`, esfera, caixa, malha)
  mapeado pela cratera no instante de cada lançamento, afastado pelo raio de colisão. Com
  raio 0,17 m: 678 → 109 → 0 de 4000 sob o chão em 2,0/3,0/5,9 s; raio padrão 0,5 mantém
  o resultado. Limite registrado no ledger: com raio 0,02 m ~1500 ficam sob o chão porque
  o colisor é malha de facetas de 1 m; abaixo da flecha da malha precisa de malha mais
  fina. Partículas (a), atrito com chão em subida: diagnóstico fechado (progressão
  geométrica, 60+ colisões por passo), correção por transporte pela superfície em curso.
- Verificação de CPU em `847d9d8`: sr-sim 341, sr-eval 346 e as 4 instâncias esperadas
  dos dois testes do mar, sr-model 94, sr-3d 82; corpus idêntico (300); clippy, fmt,
  hygiene e marcadores limpos. GPU fica para a verificação completa com a luz pela água.
- **Luz pela água integrada** (Mercurio, `f112f63`, `beb543e`, `c6195de`, merge `b883ec5`):
  domo visível atrás do vidro, absorção Beer–Lambert por `attenuationColor/Distance`,
  raio de sombra refratado para superfícies sob a água (Fresnel, cos/η², uma refinação);
  código da água em `pathtrace_water.wgsl`, emendado só em cenas com material
  transmissivo, teste do texto do shader plano; 14 testes em `water_light.rs`. Sweep de
  Mercurio na NVIDIA: 42 binários, 0 falhas; fmt e clippy 0; sonda hero 720p t=3,0 com o
  hash de referência da Fase 1. Custo UHD: oceano transmissivo 0,49 → 0,76 s de traçado;
  hero 8,66 → 8,59 s. llvmpipe dos testes novos: 14/14 em 2018 s (as três referências de força
  bruta a 1024 spp dominam); referências de força bruta no adaptador de software, decisão
  (análise de Mercurio): amostras por adaptador (1024 na GPU, 128 no software, quadro
  160×90), tolerância nunca abaixo da atual e derivada do ruído da média do retalho
  (8,15/√(spp·5000): 0,36% a 1024, 1,0% a 128 spp, contra margens de 8% e 15%),
  variável de ambiente só para forçar 1024 em qualquer adaptador; NVIDIA idêntica aos
  dígitos (0,604/0,613; 0,728/0,770). Implementado (`2cdee68`, só `water_light.rs`): a 512 spp e meio
  tamanho o ruído medido nos pixels é maior que o estimado (2,07% e 3,54%), as três
  referências caem de ~30 min para 222 s e o binário de 2018 para 407 s no llvmpipe, mas
  point/sphere ficam a 11,7% contra 15% (referência reduzida 7% acima da completa, 2
  sigma). Decisão: 1024 spp a meio tamanho (~7 min) pela margem; NVIDIA aos dígitos
  (0,6037559/0,61266124; 0,7277021 e 0,7275377/0,77031016). As "10 execuções" não se
  aplicam: render semeado por pixel, repetições idênticas ao bit. Emendado em `d9aeb32`
  (1024 spp a meio tamanho): três testes 370 s, binário 597 s no llvmpipe; erros 2,1%,
  10,0%, 10,1% contra 8% e 15%; a referência reduzida de luzes pontuais fica ~5% acima da
  completa nas duas tentativas (viés do meio tamanho, absorvido pela margem, a anotar no
  SREP). Aceito, a integrar.
- Medições de fechamento (Mercurio, protocolo aprovado): binários de `effect_costs` e
  `composite` para `60a458c` e `b883ec5`, um teste por processo em llvmpipe, 10 rodadas
  intercaladas base/topo com portão de carga medido (load1 < 4 por até 2 min, senão
  marcado); `LP_NUM_THREADS=1` e queimador de CPU só se houver diferença base/topo;
  teto 90 min; nota no SREP só com taxas e carga medidas.
  Verificação completa minha com GPU em `scratchpad/vrun_full_b883ec5.log`; parte de CPU
  já lida: sr-sim 341, sr-eval 346 e os 4 esperados, sr-3d 82, sr-model 93 e 1 falha
  `ten_thousand_nodes_load_under_200_ms` (teste de tempo, carga média 12–18 na hora, três
  sessões a compilar); a repetir isolado com a máquina mais calma antes de contar como
  regressão. Parte de GPU: sr-gpu na NVIDIA 395, CLI 61, sonda hero 720p com o hash de
  referência (traçado 14,07 s sob carga), clippy, fmt, hygiene e marcadores limpos.
- **Testes do mar com campo distante e "tudo ligado"** (Saturno, `6d3850d`, `b1552ce`,
  `3078153`, sobre `b883ec5`; a integrar quando a verificação de `b883ec5` acabar). Dois
  defeitos reais achados pelo teste combinado e corrigidos: (1) com grupo, o oceano corre
  antes das partículas e o passo que lê o respingo não as encontrava; antes do emissor se
  registar o oceano lia vazio em silêncio e o mesmo passo dava outra água ao repetir;
  agora o oceano puxa o emissor até o fim do passo (`Pull`, `Sims::advance`, `Log::knows`);
  (2) `SceneDriver::frame` ignorava problemas de `apply_physics` e lia o mundo rígido
  como parado, com 3000 ejetos diferentes por até 12 m conforme a ordem dos pedidos;
  agora o problema falha o passo fixo que perguntou. Testes do mar: onda de campo
  distante (anel 20–40 m), só ordem, faixa inteira e três pontos fora da saturação, nos
  dois modos; o que não cresce fica registrado com teste ignorado. Campo distante
  filtrado | hidrostático, por velocidade 0,95/1,32/1,85 | 0,95/1,40/1,89 m; por massa
  1,22/1,32/2,22 | 1,20/1,40/2,25 m; mergulho vertical por velocidade 1,10/1,47/2,06 m,
  por massa 1,32/1,47/2,43 m. Combinado (4 m de água, fundo+rocha, full, cavidade, 3000
  ejetos com splash): sem erro, água a 1e-9, bit a bit em qualquer ordem; 11 ejetos caem
  na água (0,20 m³). `impact_sweeps.sh` 37 passam. Custo do agendamento novo (Saturno, 144 quadros da cena
  do mar autorada, 3 execuções alternadas por binário, load1 ~11–12): 0,0646 contra
  0,0597 s/quadro, RSS 109 contra 111 MiB, dentro da dispersão de cada binário; na cena
  combinada, com e sem splash, +0,002 s/quadro e +4 MiB. Muitas partículas caindo (Saturno, 144 quadros, 3 m,
  filtrado, 3 execuções alternadas, só as de load1 6–9 lidas): com 20 m de água quase
  nenhum ejeto chega à água (34 de 3000 no pior caso, 0 na rocha autoral: nascem no fundo
  com poucos m/s); com 1 m de água 43% entram; 3000 ejetos 0,099 → 0,094–0,098 s/quadro;
  21 000 ejetos 0,217 → 0,177–0,192 s/quadro (12–19% mais rápido, as partículas saem da
  simulação); RSS igual ±2 MiB. O custo próprio do respingo não se isola sem
  instrumentação; é menor que ~0,04 s/quadro. Não medido: GPU, 1,5 m autoral com splash.
- **Partículas (a) fechado sem mudar o solver** (Netuno, `a00bd2d`): o teste de atrito 0,7
  da rocha de 270 t já passa só com o nascimento na superfície crescida (`58bfc03`); as
  partículas que estouravam as 16 colisões eram as nascidas sob a borda que sobe. O
  transporte pela superfície foi implementado, medido como desnecessário e descartado
  (proposta guardada, não enviada). Teste de regressão do caso com atrito; Saturno tira o
  `#[ignore]` do seu e declara atrito 0,7 e raio físico na cena de terra.
- **Kernel da cavidade com a profundidade da lei** (Netuno, `7e9264c`): disco esvaziado
  com R = sqrt(3V/(πd)), perfil (1-(r/R)²)² com pico = profundidade da lei e volume da
  lei, anel de R a 2R, 0,9 só como proteção. Pedido/levado 266,6/266,6, 367,8/367,8,
  862,2/862,2 (antes 0/17/28 parcelas limitadas, agora 0). Maior superfície por massa
  6,41/8,03/7,30 → 4,72/4,74/8,22 m; por velocidade → 4,17/4,74/7,52 m; mergulho vertical
  → 4,53/5,82/8,07 e 4,71/5,82/8,55 m; onda da cratera sozinha inalterada; custo nulo.
  Par frágil: 4,72 contra 4,74 por massa. A integrar depois de `3078153`; SREP com
  sobreposição a resolver por mim; `impact_sweeps.sh` na árvore combinada.
- **Crédito de pressão por dono, forma A** (Netuno, `fdfb0b0`): `Forcing::lifts` (quanto
  cada corpo ergue o fundo, esparso) e `BodySample::pressure`, soma de −g·h·∇(elevação)
  ·área·dt no fim de cada passo canônico, campo separado de `impulse`; nada aplicado,
  só creditado; sem lifts, bits idênticos. Resíduo num caso limpo (monte gaussiano móvel):
  +6% em 1ª ordem, +0,7% a +2,3% em 2ª ordem; monte parado 1,8e-4. Não bate o critério de
  0,5% que eu tinha posto. **Decisão:** forma A fica (termo de diagnóstico; a forma B
  custaria 16 B/célula e tocaria os dois fluxos); B só volta se na cena de Saturno
  impulse + pressure ficar a mais de 2% dos 45 544 kg·m/s. Saturno soma os dois no
  registro de troca depois da integração. Fila de Netuno vazia; tarefa nova: tabela de
  custo do lado do oceano na cena autoral (sem colliders; só fundo; fundo+rocha com
  none/buoyancy/full; full+cavidade; hidrostático), 144 quadros, 3 execuções com load1.
- Estimativa de completude da Fase 2 em 2026-10-05, segunda do dia (mesma rubrica): 2.0
  100% (15); 2.1 100% (15, kernel e crédito de pressão prontos, a integrar); 2.2 100%
  (10); 2.3 100% (20); 2.4 95% (10, limite de facetas da malha registrado); aceitação 90%
  (10, falta declarar atrito 0,7 na cena de terra e refazer a tabela com o kernel novo);
  render 85% (10, medições de effect_costs/composite em curso, custo do llvmpipe das
  referências); fechamento 25% (10, tabelas de custo em curso, bloco do ledger, limpeza
  do SREP, verificação final). Total ≈ 88%, contra ≈ 80% de manhã.
- **Tabela de custo do lado do oceano** (Netuno, ledger `53a9b6b`; cena autoral, 144
  quadros, CPU, 3 execuções alternadas, load1 6,0–7,1): sem colliders 0,0534 s/quadro;
  só fundo 0,0618; fundo+rocha none 0,0647; buoyancy 0,0592; full 0,0569; full+cavidade
  0,0580; hidrostático 0,0567; RSS 102–114 MiB; quadros mais lentos 0,26–0,31 s quando a
  rocha chega ao fundo. Variantes 4–7 indistinguíveis (variação de 6% entre execuções);
  a variante none fica ~10% acima; explicado por contagem (Netuno, ledger `656c399`),
  e a minha hipótese (mundo rígido avançado mais vezes sem grupo) refutada: 5 passos
  rígidos por quadro nas duas; o tempo está nos passos com a rocha em contato com a
  cratera que deforma (24–26 ms/passo contra 0,02 ms livre), e sem acoplamento a rocha
  não é freada pela água, chega ao fundo ~0,5 s antes e fica em contato o dobro dos
  quadros. Física, não ineficiência; sem correção.
- Verificação completa de `b883ec5` fechada limpa: além da CPU, sr-gpu na NVIDIA 395,
  CLI 61, sonda hero idêntica; `ten_thousand_nodes_load_under_200_ms` passa 3/3 isolado
  com load1 2,8 (0,29–0,30 s de binário), falha de carga, não regressão.
- Integração em `fa63e5d`: merges de `3078153` (Saturno) e `656c399` (Netuno), SREP
  fundido automaticamente e conferido (parágrafo da cavidade reescrito por Netuno,
  varreduras por Saturno, pressão nova). Verificação em `scratchpad/vrun_fa63e5d.log`
  (CPU, `impact_sweeps.sh`, CLI na NVIDIA, sonda). Filas: Saturno rebase, atrito 0,7
  declarado, tabela refeita, balanço impulse+pressure; Netuno rebase e revisão do SREP
  (provisórios, contradições, números sem data), lista antes de editar; Mercurio
  medições de fechamento.
- **Medições de fechamento feitas** (Mercurio, llvmpipe, base `60a458c` contra `b883ec5`,
  rustc 1.98.1, mesmas flags, logs em `r5/` do scratchpad dele): sem regressão.
  `effect_costs layers_whose_content_changes`: base 4/10 e topo 2/10 sob load1 7 (PSNR
  30,7–39,1 dB contra 40), 1/15 e 0/15 com uma thread do rasterizador; `composite
  polygons_and_stars`: base 2/50 e topo 4/50 com threads padrão (Fisher p ≈ 0,4), 0/80
  com uma thread: corrida do rasterizador de software, não nosso código. A máquina
  nunca esteve quieta (portão de load1 < 4 passou em 2 de 10 rounds). A leitura anterior
  0/3 no topo era acaso. Frase datada aprovada para o SREP e bloco no ledger, no commit
  único de Mercurio, depois de Netuno e Saturno.
- **Revisão do SREP** (Netuno, lista `srep-review.md` no scratchpad dele, 105 linhas):
  23 provisórios/obsoletos, 8 contradições, ~25 grupos de números sem commit/data, e um
  erro dele declarado (B5: "células de 1,5 m" onde eram os testes de aceitação em 3 m;
  conclusão do limite da cavidade não muda; a crista a 2,5 s em 1,5 m fica por refazer
  com o kernel atual). Decisões: `pressure` creditado e registado, não aplicado ao corpo
  nesta fase; cada número com (commit, data, load1) ou apontando o ledger; contrato no
  presente, antes/depois só no ledger. Ordem de edição para evitar conflitos: Netuno
  (factos primeiro, depois texto das secções dele e inventários), Saturno (as dele, um
  commit, depois do lote de Netuno), Mercurio (render, por último).
- **Lote de SREP de Netuno entregue** (`83d11a4` factos, `d44a0d5` texto; só SREP e
  ledger; a integrar depois da verificação de `fa63e5d`): "1,5 m" corrigido para 3 m nas
  entradas da cavidade; crista a 2,5 s refeita na cena autoral (1,5 m, código de
  `fa63e5d`): 0,81 m filtrado com cavidade, 1,41 m hidrostático com cavidade, 0,20 m e
  1,24 m sem cavidade (determinístico; a comparação com os 10,62 m do primeiro render não
  se refaz, cena e mundo rígido eram outros). Texto: 21 itens da lista fechados nas
  secções dele (contrato no presente, números com commit e data, antes/depois no ledger,
  inventários OCN1–13, P3D1–11, CRT9, scorecard por contagem, atributos em falta nas
  tabelas, teto de 64 recuperações no contrato de colisões, conferido no código). Ficam
  para Saturno A12–A14, A22, B6, L1979 e os números dele; para Mercurio os dele e S1.
  Netuno passa a revisor dos dois lotes seguintes.
- **Atrito e raio físico declarados** (Saturno, `e22458b`, a integrar): cena de terra com
  bounce 0,15, friction 0,7, collisionRadius 0,17; `#[ignore]` fora; 3748 de 4000 ejetos
  abaixo de 0,2 m/s aos 5,9 s e 0 atravessam o chão; rocha de 270 t não para o solver,
  0 atravessam (29 caem fora da borda de 80 m, contados à parte). `impact_sweeps.sh` 38
  passam (278 s). Varreduras com o kernel novo (3 m, só ordem, load1 13–15): maior
  superfície por velocidade 4,17/4,74/7,52 | 4,31/4,84/7,58 m (filtrado | hidrostático),
  por massa 4,72/4,74/8,22 | 4,79/4,84/8,42 (cresce agora nos dois); campo distante por
  velocidade 0,91/1,51/2,30, por massa 1,14/1,51/2,71; cratera sozinha inalterada.
- **Defeito de determinismo achado por Saturno:** `BodySample::pressure` não é
  reprodutível ao repetir o passo por seek do início (−0,037953 contra −0,037831; noutro
  caso ~10%), enquanto impulse, surface e centroid são idênticos. Encontrado ao gravar
  pressure no registro de troca (ramo `phase2/cosim-pressure-record`, `388ffe0`, não
  integrado, estacionado). Passado a Netuno com teste de regressão; prioridade.
- **Balanço impulso + pressão** (Saturno, cena ocean_full, bola 16 755 kg a 3 m/s, bacia
  fechada): hidrostático resíduo 3,7/3,0/10,8% em t=1/2/3 s (pressão nula, lifts só no
  filtrado); filtrado 13,3/13,2/14,9% sem pressão e 7,1/6,8/7,7% com. Os 45 544 kg·m/s
  eram a medição anterior dele em hidrostático. Passa do meu critério de 2%: a forma (B)
  volta a ser candidata; antes, Netuno decompõe a variação do momento (impulso, declive
  exato instrumentado no fluxo, fronteira, numérico) nos dois modos.
- **Determinismo da pressão corrigido** (Netuno, `09523ca`): a hipótese de Saturno estava
  certa; um reinício por checkpoint oferecia o registro do primeiro passo só com o fim
  (`from = None`) e o `pressure` saía de outra estimativa. Correção: reamostrar o início
  do passo; teste de regressão com replays e 1/2/8 threads; 19 testes do mar passam
  sobre `388ffe0`. Vai ser substituído pela forma (B).
- **Diagnóstico do balanço** (Netuno, instrumentação temporária no fluxo): a variação
  do momento da água fecha exato (< 1e-6) como empurrões aplicados + termo de fundo do
  esquema + paredes; não há resto numérico. Hidrostático: termo de fundo 3,8/3,1/11,0%
  (a elevação da ocupação, não creditada por não haver lifts); filtrado 10,5–14,5%, de
  que (A) credita só 49–65%, por ser uma correlação por diferença central; fórmulas de
  face fora do fluxo erram +17 a +25% em 2ª ordem. **Decisão:** forma (B) aprovada,
  substituindo (A) e `09523ca`: o solver acumula por dono o termo de fundo nas faces de
  fundo diferente (fluxos já calculados), ponderado pelo estágio do RK2, soma
  determinística por linha/banda, total por dono no State (vai nos checkpoints, replay
  igual por construção), lifts também no hidrostático, +16 B por dono por checkpoint,
  sem bytes por célula. Condições: bits idênticos sem lifts, 1/2/8 threads, custo no
  720×720 dentro do ruído ou < 2%, balanço refeito nos dois modos e ordens, SREP e
  ledger no mesmo lote; depois entra `388ffe0` de Saturno.
- Estimativa de completude da Fase 2, terceira de 2026-10-05 (mesma rubrica): 2.0 100%
  (15); 2.1 95% (15, forma B do crédito de pressão em curso); 2.2 100% (10); 2.3 100%
  (20); 2.4 95% (10); aceitação 100% (10, atrito declarado e varreduras com o kernel
  novo, a integrar); render 95% (10, falta o commit de SREP de Mercurio); fechamento 40%
  (10, SREP de Netuno pronto, de Saturno e Mercurio por fazer, ledger de fecho,
  verificação final, relatório). Total ≈ 90%.
- **Pacote de sign-off da fase** (decisão, a pedido do usuário): pasta fora do repositório
  `/home/pals/renders/cinematic-impact/phase2-close/` com `hero-720p.mp4`,
  `impact-land-720p.mp4`, `impact-ocean-720p.mp4`, `stills/` (hero UHD t=3,0 e quadros
  escolhidos das duas cenas), `frames/` e `MANIFEST.txt` (commit, data, GPU, tempos,
  sha256). Renderizado depois da verificação final da fase, pela fila do GPU, a partir
  do commit final de `phase2/integration`. README já escrito na pasta.
- Verificação completa de `fa63e5d` limpa: sr-sim 343, sr-eval 353 (0 falhas; os quatro
  testes do mar passam com campo distante e kernel novo), sr-model 94, sr-3d 82,
  varreduras 37/37, CLI na NVIDIA 61, sonda hero idêntica, clippy, fmt, hygiene e
  marcadores limpos.
- Integração em `90ce0f1`: merges de `09523ca` (Netuno: SREP factos e texto, determinismo
  da pressão), `d9aeb32` (Mercurio: referências de força bruta por adaptador) e
  `e22458b` (Saturno: atrito e raio declarados); fmt e hygiene limpos. Verificação
  completa com GPU em `scratchpad/vrun_90ce0f1.log`. Vez de Saturno no SREP.
- **Lote de SREP de Saturno pronto** (`98bab6c` sobre `90ce0f1`, a integrar depois da
  verificação): A12–A14, A22, B6, B2, B3/L1979, B4 fechados; números das secções dele com
  commit e data; contrato do escalonamento no parágrafo dos ejetos que caem no oceano;
  antes/depois num milestone novo do ledger com o custo do agendamento e as corridas de
  1/4/20 m de água. Decisão: `5bbab1b` (pressão no registro de troca, agora passando com
  `4c102b8`) continua estacionado até a forma (B) entrar, para o registro e o SREP não
  mudarem duas vezes.
- Estimativa de completude da Fase 2, quarta de 2026-10-05 (mesma rubrica): 2.0 100%
  (15); 2.1 95% (15, forma B em commit no branch de Netuno, `d685256`, por relatar);
  2.2 100% (10); 2.3 100% (20); 2.4 95% (10); aceitação 100% (10); render 95% (10,
  commit de SREP de Mercurio preparado); fechamento 55% (10, SREP de Netuno integrado,
  de Saturno pronto, de Mercurio preparado; faltam revisão cruzada, forma B, ledger de
  fecho, verificação final, renders de sign-off, relatório). Total ≈ 92%.
- **Forma (B) pronta** (Netuno, `d685256` código, `41f0478` SREP e ledger, sobre
  `90ce0f1`): crédito exato do termo-fonte do fundo acumulado nos sweeps de fluxo por
  dono (dono da coluna = maior elevação; face = lado mais elevado), peso do estágio do
  RK2, soma por linha/banda independente de threads, `pressure_by` no State e nos
  checkpoints (16 → 32 B por dono). Condições: bits idênticos sem lifts (teste 132×132
  paralelo); 1/2/8 threads e replays iguais ao bit (5 testes); lifts no hidrostático;
  custo 720×720 mediana de 5 rondas novo/antigo 0,92–1,01, ruído; balanço na cena de
  Saturno: resíduo 0,0000% hidrostático e 0,014–0,094% filtrado (paredes) contra
  3–15% sem crédito; defeito achado no caminho (face de wrap periódica não creditada)
  corrigido. Ressalva no SREP: termo do esquema; variação de leito sem dono (cratera)
  não creditada; continua creditado e registado, não aplicado. sr-sim 346, sr-eval 356,
  sr-model 94, clippy, fmt, hygiene limpos. A integrar com `98bab6c`.
- Verificação completa de `90ce0f1` limpa: sr-sim 345, sr-eval 355, sr-model 94, sr-3d
  82, varreduras 38/38, sr-gpu na NVIDIA 396, CLI 61, sonda idêntica, clippy, fmt,
  hygiene e marcadores limpos.
- Integração em `98b53ad`: merges de `98bab6c` (Saturno, SREP) e `41f0478` (Netuno,
  forma B com SREP e ledger); JSON do ledger válido (30 marcos), fmt e hygiene limpos.
  Verificação de CPU com varreduras em `scratchpad/vrun_cpu_98b53ad.log`; a completa
  com GPU fica para depois do commit de SREP de Mercurio ("go" dado).
- **Publicação do SREP em sr-core** (pedido do usuário, 2026-10-05): cópia de
  `srep-0000-cinematic-impact.md` (texto de `0b90305`, o commit de SREP de Mercurio) em
  `/home/pals/src/sr-core/srep/srep-0000-cinematic-impact.md`, número 0 como manda o
  SREP 0 para rascunhos (precedente: `srep-0000-force-field-scope.md`, fora do índice);
  os quatro links para ficheiros do motor viraram caminhos simples para passar o hygiene
  do sr-core. Push autorizado pelo usuário e feito: `f10ef10` em `origin/main` do sr-core (rebaseado
  sobre `fcf3b64`, um rascunho de outro autor que entrou entretanto); texto de `a8a3945`.
- **Revisão de Netuno sobre 0b90305** (Mercurio): nenhuma contradição; quatro itens
  pequenos a emendar antes da integração (370 s/597 s em vez de "~30 min"; "dois commits
  medidos"; "uma thread do llvmpipe"; frase do domo 4 contra 24 bounces com teste e
  ledger; "cena do oceano"). Sobre `98bab6c` (Saturno): tudo fechado menos o parágrafo
  duplicado da pressão (L2077-2086, desatualizado depois de B), o `limits` da entrada
  dele no ledger e um comentário de teste; vão com `5bbab1b`. A mensagem com esses itens
  para Saturno foi negada pelo classificador; a reenviar quando o usuário autorizar ou
  por outra via aprovada.
- **Estimativa dos renders de sign-off** (Mercurio): 3 × 144 quadros a 720p + 1 UHD;
  15–35 min de fila se a sequência simular incrementalmente, até 90 min se cada quadro
  ressimular; ordem aprovada: calibração (5 quadros hero + land/ocean em t=5,9), UHD
  hero, oceano, terra, hero em 4 segmentos de 36 quadros; nenhuma posse da fila acima de
  10 min; "go" depois da verificação final.
- **Balanço confirmado com a forma exata** (Saturno, ramo estacionado `9fc0d69`, pressão
  no registro de troca comparada ao bit): hidrostático resíduo 0,00% em t=1/2/3 s;
  filtrado 0,01/0,04/0,09%; bate com Netuno. Erro de contagem dele na primeira versão
  do teste (registros 0..passos−1) declarado e corrigido; o teste afirma |resíduo| <
  0,5% e menor que sem a pressão nos dois modos. Patch de SREP pronto no scratchpad dele,
  a aplicar depois de `a8a3945`.
- Verificação de CPU de `98b53ad` limpa (sr-sim 346, sr-eval 356, sr-model 94, sr-3d 82,
  varreduras 38/38, clippy, fmt, hygiene, marcadores). Integração em `fc3f82e`: merge de
  `a8a3945` (Mercurio, SREP de render e ledger, emendado após a revisão de Netuno); JSON
  válido (33 marcos), hygiene e fmt limpos. Falta só o commit pequeno de Saturno (código
  `9fc0d69` rebaseado + texto); depois: verificação final com GPU, renders de sign-off,
  relatório de fechamento.
- Integração em `fd3c8f0`: merge de `1ba58cd` (Saturno: `Around.pressure` no registro
  de troca comparado ao bit, `Evaluator::around_into`, teste do balanço exato) e
  `56314ea` (texto); o exemplo descartável que tinha entrado por engano foi retirado por
  emenda. Último código da Fase 2. Verificação final completa com GPU em
  `scratchpad/vrun_fd3c8f0.log`. Pendentes só de texto: os três itens de Saturno (à
  espera de autorização do usuário para o reenvio da mensagem bloqueada). A cópia
  publicada no sr-core (`f10ef10`, texto de `a8a3945`) fica a atualizar no fecho com o
  texto final.
- Revisão final de Netuno sobre `56314ea`: o SREP está fechado (parágrafo duplicado da
  pressão saiu, números batem). O ledger de Saturno ficou desatualizado (resíduo "em
  diagnóstico", 388ffe0 e os 7,1/6,8/7,7% da estimativa, sem entrada para `1ba58cd`).
  Decisão: Netuno faz o commit pequeno de ledger (corrigir os dois campos, entrada nova
  de `1ba58cd`, e o seu aviso D1 em :2897); Saturno fica só com o comentário do teste.
- Ledger corrigido (Netuno, `9947426`, só JSON, a integrar depois da verificação final):
  limits e validação da entrada de Saturno com os resíduos exatos e `1ba58cd`; entrada
  nova "The pressure of the exchange is recorded and balances the water" com os números
  por extenso (cena: bacia fechada 160 m, células de 2 m, 10 m de água, bola 16 755 kg a
  3 m/s, full, passo 0,05 s, 1ª ordem, load1 ≈ 9); aviso D1 de :2897 corrigido. Nenhum
  teste lê o ledger (grep), por isso basta hygiene e JSON sobre o merge. Calibração dos
  renders (a) autorizada enquanto a fila está livre; (b)–(e) após a verificação.
- **Calibração dos renders** (Mercurio, binário `fd3c8f0`, jobs curtos): a sequência
  evolui incrementalmente (terra, quadros 72–75 num processo: o primeiro paga a simulação
  inteira, 2,3 + 3,2 + 4,3 s, os seguintes ~0,5 s cada). Terra t=5,9: partículas 7,8 s,
  rígido 4,2 s, fumaça 8,7 s, traçado 0,56 s. Oceano t=5,9: oceano 6,9 s, rígido 7,0 s,
  traçado 0,05 s. Hero 720p: traçado 0,01/2,78/10,12/10,29/9,22 s em t=0,5/1,5/3,0/4,5/5,9
  (hash de t=3,0 igual à referência). Estimativa: oceano ~1 min, terra ~2 min, UHD hero
  1,5–2 min, hero 720p 17–19 min em três segmentos (0..60, 60..102, 102..144) para não
  passar de 10 min por posse da fila; total ~25 min. Ordem: oceano, terra, UHD, hero A/B/C,
  codificação e manifesto fora da fila; stills das cenas de impacto vêm dos quadros já
  renderizados.
- Estimativa de completude da Fase 2, quinta de 2026-10-05 (mesma rubrica): 2.0 100%
  (15); 2.1 100% (15, forma B e registro na troca integrados); 2.2 100% (10); 2.3 100%
  (20); 2.4 95% (10); aceitação 100% (10); render 100% (10); fechamento 70% (10, SREP
  fechado, ledger a integrar, verificação final em curso com varreduras 38/38, faltam
  renders de sign-off, relatório, comentário de teste de Saturno, cópia final no
  sr-core). Total ≈ 96%.
- Verificação final de `fd3c8f0`: o binário de teste `backends` do sr-gpu corre só na CPU
  (adaptador de software, GPU a 0%) e segurou a fila do GPU por mais de 40 min. Como
  `crates/sr-gpu` é byte a byte igual a `90ce0f1`, onde a suíte inteira do sr-gpu (396,
  com `backends`) passou, interrompi só esse binário para a verificação seguir e a fila
  ficar livre para os renders; o resto do sr-gpu, a CLI e a sonda correm normalmente.
  Registro: `backends` não repetido em `fd3c8f0`, por identidade do código.
- **Verificação final de `fd3c8f0` limpa** (2026-10-05 22:20 UTC): sr-sim 346, sr-eval
  357, sr-model 94, sr-3d 82, varreduras 38/38, sr-gpu na NVIDIA 392 (sem os 4 testes de
  `backends`, não repetidos por identidade do código com `90ce0f1`), CLI 61, sonda hero
  720p t=3,0 com o hash de referência da Fase 1, clippy, fmt, hygiene e marcadores
  limpos. "Go" dado a Mercurio para os renders de sign-off.
- Integração em `5e6ce7d`: merge de `9947426` (ledger, Netuno); JSON válido (34 marcos),
  hygiene e fmt limpos. Só documentação acima de `fd3c8f0`, o código dos renders.
- **Renders de sign-off, oceano entregue** (Mercurio, binário `fd3c8f0`, 144 quadros num
  job curto de 62 s): `impact-ocean-720p.mp4` (h264, 1280×720, 24 fps, 144 quadros,
  conferido por ffprobe) e `frames/impact-ocean/`. Conferi os quadros 60 e 120: anel da
  cavidade e ondas em expansão; a faixa pontilhada no horizonte do plano de mar distante
  e os pontos escuros nas bordas do anel são os defeitos menores já anotados para a fase
  de render (8 amostras com denoise, plano `farSea`), não da física.
- **Renders de sign-off, terra e UHD entregues** (Mercurio): `impact-land-720p.mp4` (144
  quadros em 81 s, conferido por ffprobe) e `stills/hero-uhd-t3.0.png` (3840×2160, 131 s de
  parede, RSS 677 MiB). Conferi o quadro 100 da terra: cúpula de poeira, ejetos espalhados
  em anel, cratera pouco visível da câmera rasante. Defeito pequeno de estatísticas achado
  por Mercurio: num quadro UHD em tiles, `pathtrace trace` reporta 0,4 s contra 131 s de
  parede (só o último tile?); o manifesto usa só a parede. Item para a Fase 4 (estatísticas
  por tile somadas).
- **Pacote de sign-off completo** (2026-10-05 22:46 UTC, Mercurio; 212 MB; 23,4 min de
  fila, nenhum job acima de 8 min): três vídeos de 144 quadros a 24 fps, UHD da hero,
  8 stills, `MANIFEST.txt` com 445 sha256; conferi todos os hashes (445/445) e o quadro
  72 da hero com o hash de referência; costuras dos segmentos sem diferença. Quadro 120
  da hero: pluma cortada em caixa no topo do domínio, limite conhecido desde a Fase 1
  (domínio da fumaça fixo). `REPORT.md` escrito na pasta.
- **Fase 2 fechada em `5e6ce7d`** (código `fd3c8f0`). Abertos só de texto: comentário do
  teste de Saturno (mensagem bloqueada) e a cópia do SREP no sr-core com o texto final.
- **Relatório de fechamento revisto por Netuno** (12 itens, todos aplicados em
  `REPORT.md`): dois eram afirmações minhas acima do medido ("balanço exato" onde é
  0,09% no filtrado; "defeitos já anotados" onde só estavam no plano, não no SREP nem no
  ledger); os outros: custo do oceano acoplado 0,057–0,059 s/quadro (não 0,053–0,065),
  cadeia inteira só num teste de 4 m de água, números do render de medições diferentes
  separados, T da cavidade ≈0,7 s calculado, hygiene com 12 avisos D1, backends não
  repetido, determinismo por solver, cinco defeitos achados (não três), 108 commits,
  limites (3 m e só ordem; kernel, bodyDrag e atrito são escolhas do motor; 8 spp).
  Achado dele: dois mp4 soltos na raiz do checkout principal (saída do ffmpeg de
  Mercurio com cwd errado); passados a Mercurio para apagar.
- **PAUSA por ordem do usuário (2026-10-05 ~23:10 UTC).** Estado para retomar:
  - `phase2/integration` = `5e6ce7d` (Fase 2 fechada; código verificado em `fd3c8f0`);
    worktree `realism` limpo, só este plano não rastreado. `main` e
    `cinematic-impact-checkpoint` intocados (`60a458c`); nada enviado ao remoto.
  - Branches dos agentes: `phase2/cosim` e `phase2/ocean-bed` em `5e6ce7d`;
    `phase2/volume-light` em `fd3c8f0`; `phase2/cosim-pressure-record` integrado;
    `parked/ocean-sponge` `b727f6b` continua estacionado. Relatórios de pausa dos
    agentes abaixo quando chegarem. Fila do GPU livre; nenhum job meu a correr.
  - Pacote de sign-off completo em `/home/pals/renders/cinematic-impact/phase2-close/`
    (REPORT.md revisto por Netuno; MANIFEST 445/445 conferido). Cópias dos vídeos
    apareceram na raiz do checkout principal depois do pacote, provavelmente do
    usuário; Mercurio apagou duas idênticas antes de eu mandar parar; `hero-720p.mp4`
    ficou. Não mexer.
  - sr-core: `srep/srep-0000-cinematic-impact.md` publicado em `origin/main` como
    `f10ef10` (texto de `a8a3945`); commit local `8a3f02e` com o texto final (de
    `5e6ce7d`) NÃO enviado, à espera de autorização do usuário.
  - Pendentes de texto: comentário do teste `in_the_sea_the_highest_surface_anywhere_is_recorded`
    em `impact_scenes.rs` (Saturno; a mensagem com o item foi bloqueada pelo
    classificador; o item está em `srep-review.md` de Netuno, no scratchpad dele, e no
    parecer dele acima); avisos D1 antigos no ledger (:1465, :1539) e em comentários de
    código (exchange.rs, lift.rs, physics3d.rs); os quatro defeitos visuais dos renders
    não estão no SREP/ledger (só em REPORT.md e aqui).
  - Para retomar: pedir status aos três agentes (nomes de sessão mudam a cada reinício;
    conferir com ListAgents e um pedido de identificação), decidir o push de
    `phase2/integration` (usuário), e começar a Fase 3 pelo plano da secção 4 mais os
    itens acrescentados nesta fase (colisor da cratera barato, domínio da fumaça que
    acompanha a pluma, escala de tempo uniforme).
- **Relatórios de pausa dos agentes (2026-10-05):**
  - Netuno: `phase2/ocean-bed` = `5e6ce7d`, árvore limpa, nada a correr, nada pendente.
    Ficheiros temporários no scratchpad da sessão 7ba1ee78 (`srep-review.md`, diffs
    revistos, `balance.rs`, binários da medição de custo, `trace-instrumentation.patch`).
  - Mercurio: `phase2/volume-light` = `fd3c8f0`, só o helper `crates/sr-gpu/examples/imgcmp.rs`
    não rastreado, nada a correr, nada pendente. Scratchpad da sessão 8e0c69e6: r1–r6
    (medições, binários, logs da fila), `srep/apply.py`, `scene-render-fd3c8f0`.
    Avisos dele para quem retomar: estimar GPU antes de cada job e ficar abaixo de
    10 min; dizer sempre o adaptador; `backends` nunca termina; abertos na área dele sem
    agenda: cáusticas, câmera sob a água, segunda interface no raio de sombra, `farSea`
    transmissivo, pontos no horizonte a 8 spp, manchas de baixa frequência no chão.
    Apagou duas cópias idênticas dos vídeos na raiz do checkout principal; `hero-720p.mp4`
    (cópia idêntica, 22:52) ficou e não é nosso para tocar.
  - Saturno: `phase2/cosim` = `5e6ce7d`; `phase2/cosim-pressure-record` (`56314ea`) já
    integrado, obsoleto, pode ser apagado (ele não apaga); só `sea_cost.rs` não rastreado
    (cópia em scratchpad da sessão 6e5c014b, com as cenas de custo, `closing-acceptance.md`,
    `sweeps2.txt`, `bal2.txt`). Pendentes dele: (a) reescrever o comentário do teste
    `in_the_sea_the_highest_surface_anywhere_is_recorded` (~linha 591) como "registrada,
    não afirmada, o primeiro par difere 0,4%" (ele próprio identificou o item, o que
    resolve a mensagem bloqueada); (b) decisão minha sobre `sea_cost` como ferramenta;
    (c) refazer a tabela das varreduras se o kernel, a resposta do leito ou as partículas
    mudarem.
- **Pedido do usuário (2026-10-05, depois da pausa): commit em `main` e push.** Estado
  encontrado: `main` local em `f296a4c`; `origin/main` em `755beb7` com 10+ commits de
  outros autores (65 ficheiros, +4902) que não estão em `5e6ce7d`; `5e6ce7d` descende de
  `60a458c` (= `cinematic-impact-checkpoint`), que descende de `f296a4c`. Fast-forward
  impossível; fiz um merge em branch `main-next` = `origin/main` + `5e6ce7d`. Um conflito,
  em `crates/sr-eval/src/lib.rs` (exports), resolvido pela união. `cargo check` do
  workspace limpo. Mensagens dos 185 commits passam o hygiene `--commit-msg`. Corpus
  regenerado: 305 documentos, oráculo de acordo; `invalid/c65` muda porque a cópia
  commitada no upstream tinha derivado do gerador (172 documentos usam `target="title"`,
  só c65 tinha `title-layer`). `cargo fmt --check` falhava em 8 ficheiros do upstream,
  não tocados por nós: commit separado `style: rustfmt`. Verificação completa (CPU, GPU,
  varreduras, sonda) a correr sobre `main-next` antes do push.
- **Push feito por ordem do usuário (2026-10-05, antes do fim da verificação):**
  `main` do rs-scene-render movido para `d823d28` (merge `6b71007` + rustfmt) e enviado,
  `origin/main` 755beb7 → d823d28. sr-core: `8a3f02e` rebaseado sobre 17 commits novos
  do remoto (nenhum tocava o rascunho) e enviado como `6e59cc7`. A verificação completa
  de `d823d28` continua a correr (sr-sim 346/0 já; o resto por vir); se falhar, a
  correção vai em commit novo sobre `main`.
- **Verificação de `d823d28` (main enviado):** sr-sim 346, sr-eval 393, sr-model 101,
  sr-3d 82, varreduras 38/38, sr-gpu na NVIDIA 447/0, CLI 73 e 1 falha, sonda hero
  idêntica, fmt e hygiene limpos; **clippy falha** e a CLI tem uma falha, ambas de origem
  do upstream: (1) `items_after_test_module` em `sr-gpu/src/fx.rs` (`hue_of_linear` depois
  de `mod tests`, commit upstream 09acc45) e `render.rs` (`project_25` depois de
  `mod draw_uniform_tests`, aa93e09); já assim em `755beb7`, isto é, o clippy do upstream
  já falhava; correção: mover as funções para cima do módulo de teste, commit novo em
  `main`. (2) `a_flat_document_still_renders_on_opengl` (teste do upstream, f6dd379):
  com `SR_GPU_BACKEND=gl` o adaptador GL existe (`Gl NVIDIA RTX 6000 Ada ... Other`) mas
  a criação do device falha com "Parent device is lost"; falha igual com o binário
  `fd3c8f0` de antes do merge e com o binário já existente em
  `/home/pals/src/rs-scene-render/target/release/`; ambiente desta máquina (GL sem
  display/EGL), não o merge; fica registado para o autor do teste (o teste só salta
  quando não há adaptador GL, não quando o device falha).
- Correção do clippy em `main`: `0bc940c` ("the test modules close their files"), só
  movimento de código em `fx.rs` e `render.rs`; clippy do workspace limpo, 45 testes de
  lib do sr-gpu passam, fmt e hygiene limpos; `main` enviado, `origin/main` = `0bc940c`.
  Fica em aberto, do upstream: o teste `a_flat_document_still_renders_on_opengl` falha
  nesta máquina por "Parent device is lost" na criação do device GL (ambiente), em
  qualquer binário.
- **Revisão do código (2026-10-06, pedido do usuário), sr-gpu, confirmado por mim no
  fonte:** (1) MAJOR, `pathtrace.rs:838-847` + `pathtrace_water.wgsl:76`: o ramo da luz
  pela água não multiplica pelo cosseno de Lambert (n·dir) nem divide por cos θ_w na
  interface; o ramo plano tem `* nl`. Os dois fatores só se cancelam para superfícies
  paralelas à interface (o chão plano dos testes), por isso todos os testes passam; uma
  face vertical ou o declive de uma cratera sob a água recebe irradiância errada (dezenas
  de %). (2) MAJOR, `render_three.rs:1959` e XSD `light@castShadow` default `false`: o
  ramo da água só corre quando a luz projeta sombra (`lt.size.y > 0.5`); uma luz com o
  default dá ao chão submerso a irradiância seca, sem Fresnel, cos/η² nem absorção; os
  testes escrevem sempre castShadow="true"; as cenas de aceitação também. (3) MAJOR,
  pré-existente, `pathtrace.wgsl:640-643`: Schlick na saída água→ar usa o cosseno do lado
  denso; entre ~40° e a reflexão total (48,6°) subestima F (0,02 contra 0,14 a 45°); as
  referências de força bruta saem da água por esse caminho, logo estão enviesadas no
  mesmo sentido. Menores: η² só num dos lados (certo só com câmera e luzes no ar);
  referências de força bruta não independentes (mesmo shader); `nodes as u32` pode
  saturar antes do limite da grade; cópia literal de `volume_incident_exact`; testes de
  força bruta caros por defeito no adaptador de software; sombra volumétrica perdida no
  raio refratado; segunda interface sob ondas não re-refratada. Sólido: splicing com
  âncoras verificadas, lobo especular consistente, indexação e orçamento da grade.
- **Revisão do código, esquema/regras/ferramentas/CLI, confirmado por mim:** (1) MAJOR,
  CRT4 (`rules.rs:673-675`, `.sch` linha 55): o envelope `influenceDepth ≥ 2·max(depth,
  rimHeight)` usa o default `depth=10` quando a cratera tem `source` (CRT6 proíbe
  `depth` autoral), então rejeita `influenceDepth="5"` numa cratera da lei de 1 m e aceita
  `influenceDepth="30"` numa de 16 m, que falha no render (`sr-3d/crater.rs:52`);
  correção: aplicar o envelope só sem `source`, ou pôr `influenceDepth` em CRT6. (2)
  MAJOR (ferramenta), `check_release_hygiene.py --staged` lista os ficheiros do índice
  mas lê o conteúdo da árvore de trabalho (linhas 77-99): um achado em stage com a
  correção não adicionada passa; corrigir lendo `git show :PATH`. Menores: C1 não apanha
  `phase2/...` em minúsculas (36 mensagens de merge desta série); D1 com 7 de 9 falsos
  positivos e a varrer `vendor/`; `bodyDrag` sem default no XSD; mensagem de P3D10 não
  diz o spread default 15; "um emissor pertence a um oceano" é rejeitado pelo compilador,
  não por OCN13 como o SREP diz; `changes.rs` não rastreia os quadros de volumes
  `srvseq`; velocidade autoral em `pyroSource@crater` aceita sem o contrato dizer; B5
  trata `//` dentro de strings como comentário. Conferido consistente: defaults de
  bedResponse, bodyCoupling, heatFraction/dustFraction/specificHeat/maxTemperature,
  lightGrid*, maxWork, capture, collisionRadius, W02, solver/advection, bake, listas de
  PYC1/OCN10/CRT6 iguais em sch e Rust, sem ids duplicados, corpus igual ao oráculo
  (305), CLI nunca sai com 0 em erro.
- **Revisão do código, sr-sim (revisor; itens 1 e 7 conferidos por mim no fonte):** (1)
  MAJOR, `physics3d.rs:467,1015-1023`: enquanto há uma vigia de impacto pendente, cada
  passo recolhe os contatos de TODOS os pares com um teto fixo de 4096 pontos
  (`WATCH_CONTACTS_PER_STEP`), embora só os pares vigiados sejam lidos; um campo de
  destroços ou fragmentos sobre terreno em malha pode estourar o teto, o mundo restaura
  o checkpoint, repete e falha igual, e todos os quadros seguintes voltam com erro; sem
  teste do teto; correção: filtrar os pares pela vigia antes de contar, teto configurável.
  (7) menor, `particles3d.rs:578-587`: uma partícula cuja morte cai em (lo, hi] não é
  integrada nem testada contra a água, então um ejeto que cairia na água antes de morrer
  nunca entra no respingo (massa perdida; os testes passam porque a vida é maior que a
  cena). Outros menores do revisor, não conferidos por mim: pico de memória da exportação
  de bricks ≈2× o declarado; `seek` clona os dois `Forcing` enquanto `ends` ainda os guarda;
  área da linha de água da esfera vs malha em declive (fator √(1+s²)); `lift.rs` sem teto
  em depth/cell e 16 MB de scratch não orçados; `faces.push` sem capacidade; `project`
  com dois laços de CG quase iguais e MacCormack re-traçando a característica da passagem
  1; doc de trabalho por célula desatualizada para 2ª ordem; "uma coluna uma vez" não
  validado em `sample`; só a primeira vigia por dono alimenta `Driver3::surface`;
  `Error::Pressure` sem célula ofensora. Sólido: multigrid (Galerkin, V(1,1), componentes
  flutuantes, Cholesky), reduções independentes de threads com testes 1/2/8, MUSCL/RK2 e
  faces, unidades.
- **Revisão do código, sr-eval (itens 1 e 2 conferidos por mim):** (1) MAJOR,
  `splash.rs:192`: `local_time = a.time + emitter.start - ocean.start`, mas `a.time` já
  está no relógio do emissor que começa em `emissionStart` (sr-sim `spec.start`) e
  `emitter.start = own.start + emissionStart` (`particles3d.rs:127`): emissionStart somado
  duas vezes; com `emissionStart="1"` o respingo é arquivado 24 passos à frente e perde-se
  em silêncio; nenhum teste de respingo usa emissionStart. (2) MAJOR, `pyro.rs:339-343,
  355-358`: os problemas de `apply_physics` não são verificados (o oceano e as partículas
  verificam): um mundo rígido que falhou é lido como "sem impacto ainda" e corpos nas
  poses autorais, o passo é guardado e um replay posterior vê outras entradas (fumaça
  dependente da ordem; poeira/calor do impacto podem nunca entrar). Revisor, não
  conferidos por mim: (3) desempenho, o oceano não guarda o estado anterior ao
  avanço-além e `seek` reinicia do checkpoint, replay de até 60 passos por quadro quando
  reach − dt ≳ 1/fps; (4) `hold` da fumaça com 2 entradas insuficiente com gas + splash
  (passos simulados duas vezes); (5) emissor ausente do grafo no primeiro cálculo de um
  passo → `read` devolve vazio; (6) `Exchange` log morto ao lado de `Around`; (7)
  `Sims::apply` ~210 linhas, 8 allow(too_many_arguments); (8) sonda do relógio de
  composição e arredondamento do passo canónico triplicados, `pull` construído duas vezes,
  bodyDrag/waterLevel/gravity analisados em dois sítios; (9) cascos de empuxo construídos
  para colisores estáticos; (10) identidade do cache completa (nada em falta), mas via
  `Debug` de `Shape3` (O(malha) por bake); (11) um registo por passo em vez de dois.
  Sólido: convenções de passo alinhadas (verificadas pelo revisor), registro de troca
  testado para independência de ordem.
- **2026-10-06, ordem do usuário: "fix all, apply TDD".** Pausa levantada. Base: `main`
  = `0bc940c`. Atribuições enviadas: Netuno N1–N11 (splash emissionStart, emissor
  ausente, avanço-além do oceano, log Exchange morto, duplicações, cascos estáticos,
  clone em seek, lift.rs, unicidade de lifts, hidrostática da esfera, doc de trabalho);
  Saturno S1–S13 (pyro lê física falhada, teto de contatos, partícula que morre no passo,
  vigias por dono, memória da exportação, faces, CG/MacCormack, Error::Pressure, hold,
  CRT4, menores do esquema, ferramenta de hygiene, srvseq); Mercurio M1–M12 (cosseno de
  Lambert com oráculo independente numa face vertical, castShadow, Fresnel de saída, η²,
  retalho vertical, overflow da grade, custo da força bruta, cópia do shader, docs,
  render_timed, sombra volumétrica, re-refração). Regra: teste que falha primeiro, um
  commit por item, mensagem no presente; eu integro em `main` local, verifico e envio.
  Worktree `realism` agora em `main`.
- **2026-10-06, pedido do usuário: vídeo de 30 s em pixel art do impacto, vários POV.**
  Decisão: produção sobre o motor, sem código novo: a cena de terra (6 s, determinística)
  filmada de 5 câmeras × 6 s (aproximação geral; nível do chão junto à cratera; de cima;
  contra-plano baixo a contraluz; geral afastando-se), render nativo 320×180, 8 spp sem
  denoise, posterize/quantize do formato para a paleta, ampliação por vizinho mais próximo
  para 1280×720, 24 fps, libx264. Mercurio executa (prioridade sobre M1–M12): passo 1
  pré-visualizações de um quadro por câmera para aprovação; passo 2 as sequências,
  concatenação, MANIFEST em `/home/pals/renders/cinematic-impact/pixelart/`. Refinado
  pelo usuário: estilo adventure game dos anos 90: 320×200 ampliado ×4 para 1280×800,
  paleta VGA (64 cores; variante EGA de 16 para escolha), legenda narrativa por plano em
  português na faixa inferior, panorâmicas lentas nos planos 1 e 5, cortes secos.
- Pré-visualizações do pixel art (Mercurio, 320×180, 8 spp, LUT de 25 cores com
  color-grade; posterize dá salpicado colorido no chão por causa do ruído num limiar):
  pov2 (nível do chão) e pov4 (contraluz) aprovados; pov3 aprovado; pov1 a aproximar
  (z=-70) e horizonte mais baixo; pov5 a estreitar para esconder a faixa do mapa de
  ambiente. Chão salpicado em todos: pedido teste 32 spp com e sem denoise. Paleta: LUT
  interpolada como base; variante EGA com posterize depois. Física ressimula por cópia
  (cache inclui o documento): 25 s por POV; passo 2 estimado em ~7 min de fila. Itens
  M1–M5, M4, M12 da revisão já commitados em `fix/render-review` (89f5ff4…9b67c85),
  pendentes M11, M6, M8, M7, M10, M9.
- Pixel art, provas em 320×200 vistas e decididas: 32 spp com denoise (chão plano, ejetos
  nítidos); paleta estrita de 16 cores (LUT em 4 passagens; a EGA verdadeira sem tons de
  areia apaga a poeira); legendas em DejaVu Sans Mono Bold 9 px legíveis; plano 5 passa a
  usar t=3–9 s de uma cópia com duration 9 s para "Depois, o silêncio." cair sobre a
  poeira a assentar. "Go" ao passo 2: cinco jobs curtos, ~12 min de fila estimados,
  saída `impact-pixelart-30s.mp4` a 1280×800.
- **Vídeo em pixel art entregue** (Mercurio, 2026-10-06): `impact-pixelart-30s.mp4`,
  1280×800, 24 fps, 720 quadros, 30,000 s por ffprobe; cinco planos de 144 quadros, cortes
  secos; 8,9 min de fila, job mais longo 2,3 min; MANIFEST com 775 sha256 conferidos por
  mim (775/775). Pasta com shots/, scenes/ (cinco cenas, LUT, sky.hdr), previews/.
  Observação dele: no plano 5 a coluna de poeira cresce e sai pelo topo do quadro perto de
  t=8 s. Mercurio volta aos itens M11, M6, M8, M7, M10, M9.
- **2026-10-06, pedido do usuário: buraco negro em pixel art.** Sem lente gravitacional
  no motor (seria SREP próprio); encenação com o formato: campo de estrelas com efeito
  bulge/twirl a fingir a lente, disco preto com anel emissivo, disco de acreção em
  particles3D com forceField vortex + point, glow, paleta de 16 cores com rampa quente,
  3 planos × 10 s. Mercurio: provas primeiro (com e sem o efeito de lente), depois
  sequências em `/home/pals/renders/cinematic-impact/pixelart-blackhole/`.
- Buraco negro, provas vistas: três planos aprovados com a lente fingida (bulge sobre
  traços tangenciais pré-desenhados no PNG de estrelas; sem o efeito lêem-se como riscos);
  anel de fótons em toro emissivo, esfera preta unlit, disco em particles3D com forceField
  radial + vortex (vortex só gira no plano xy: câmera 20° acima), duas emissoras com cor
  por idade, glow fraco, LUT em 8 passagens para 16 cores estritas. Custo medido ~7 s por
  quadro: pedido o detalhe por estágio e testes a 16 e 8 spp antes do passo 2 (85 min
  estimados a 32 spp, em 36 jobs ≤ 3 min). Dois achados de motor para Saturno (S14:
  particles3D com dt = 1/24 falha "particle spin time or angular velocity"; S15:
  emitterMesh plano não emite nem erra).
- **Buraco negro em pixel art entregue** (Mercurio): `blackhole-pixelart-30s.mp4`, 1280×800,
  24 fps, 720 quadros, 30,000 s (ffprobe meu; sha256 041ca1a2…); 3 jobs de 240 quadros,
  34–46 s cada. Custo explicado: a GPU gasta 37–65 ms por quadro (traçado 97%); os ~7 s
  eram o arranque e a simulação do primeiro quadro de cada job; dentro do job a simulação
  avança incrementalmente. 16 spp escolhido (8/16/32 iguais à vista; 3,4% dos pixels
  diferem por um degrau de paleta). MANIFEST declara a lente como encenação e as
  substituições. Itens S14/S15 para Saturno com a cena de reprodução
  (`scenes/bh1_final.scene.xml`, dt=1/24, quadro 48).
- **Correções da revisão, lote de Saturno (S1–S13) entregue e fundido em main local**
  (`fix/cosim-review` 1c1d145, 14 commits, teste que falha primeiro em cada): S1 erro da
  fumaça quando a física falha (`pyro::Error::Driver`); S2 só os pares vigiados contam
  para o teto, teto configurável; S10 CRT4 só sem `source` (corpus nos dois sentidos);
  S3 partícula que morre no passo ainda cai na água; S4 dono recebe o impacto mais cedo
  entre as vigias; S9 causa real era a ordem especial com `gas` (fumaça antes das
  partículas): ordem uniforme, 30 passos em vez de 69; S12 hygiene lê o índice, C1 sem
  caixa e `phase-?N`, D1 9 → 0 avisos, B5 ignora `//` em strings, 10 testes Python; S5
  NÃO reproduz (pico 18,1 MB contra 18,8 declarado), teste de regressão fica; S6 faces
  com capacidade exata (29,6 → 15,2 MB = cobrado); S8 `Error::Pressure` com a célula pior;
  S13 quadros de `srvseq` como inputs; S11 defaults e contratos documentados (pyroSource
  velocity fica autorável e documentado); S7 MacCormack traça uma vez (advecção 470–562 →
  404–471 ms) e um só laço de CG, hashes de referência iguais. S14/S15 não reproduzidos
  ainda (precisam da cena de Mercurio). Verificação de CPU em curso sobre o merge.
- **2026-10-06, ordem do usuário: "implemente o que precisa e renderize uma simulação"
  (buraco negro físico).** Mini-fase "buraco negro de Schwarzschild", desenho meu: G = c
  = 1, M em unidades da cena, sem rotação; elementos `blackHole` e `accretionDisk`,
  `camera@geodesics`; câmera geodésica em WGSL (Binet, RK4 em φ: captura, escape para o
  fundo, cruzamentos do plano do disco incluindo imagens de ordem superior); disco fino
  analítico kepleriano com T(r) de Shakura–Sunyaev, corpo negro, padrão azimutal semeado
  que roda com Ω = √(M/r³), redshift de Luminet (Doppler + gravitacional), I × g⁴; outros
  objetos são erro de validação nesta versão. Oráculos de aceitação: sombra √27 M,
  deflexão exata vs b e 4M/b, ISCO 6M, g = √(1−3M/r) no azimute nulo, simetria, shader
  igual à referência em Rust a 64×40, imagem secundária a 75°, custo < 1 s por quadro a
  1280×800. Divisão: Netuno `sr-sim/gr.rs` (integrador + fórmulas + testes); Mercurio
  câmera e disco no sr-gpu + render final (30 s a 1280×800 e versão pixel art) em
  `/home/pals/renders/cinematic-impact/blackhole-gr/`; Saturno XSD 1.3, Schematron BH*,
  rules.rs, corpus, secção do SREP. Branches `feat/blackhole-gr*` sobre main; TDD; eu
  integro. Os itens restantes da revisão (N3–N11, M6–M10, S14/S15) ficam atrás.
- MANIFEST do buraco negro em pixel art completado (750 hashes). Nota corrigida de
  Mercurio sobre S14: o dt=1/24 falha nos quadros em t múltiplo de 0,5 s (12, 24, 36, 48,
  72, 144, 264), passa em 0, 25, 47, 49; repro em `scenes/repro-dt/bh1_dt24.scene.xml`.
- Decisões de projeto de Mercurio no buraco negro, aceitas: RK4 em φ com passos que
  aterram exatamente nos cruzamentos do plano do disco (φ0 + kπ, forma fechada); disco
  opaco de primeiro acerto com até 4 cruzamentos (imagem secundária como em Luminet);
  g = √(1−3M/r)/(1−Ωλ), λ = b·(ĥ·ẑ); cor de corpo negro a g·T, luminância ∝ (gT/T_pico)⁴;
  fundo v1 procedural com anti-aliasing por amostras. Tolerância shader (f32) contra
  referência (f64): ≤ 2e-4 relativo, 1e-6 se houver referência f32 de ordem igual.
  Atributos extras combinados com Saturno: contrast, intensity, timeScale,
  rotationX/rotationZ, angularPattern none|clumps|spiral, temperatureScale em K; BH6
  proíbe object3D/partículas/meios com geodesics; BH7/BH8. M6 fechado (d28dbf1).
- **Correções da revisão, lote de Netuno (N1–N11) entregue** (`fix/ocean-review`
  4fbc5be, 12 commits, teste antes de cada): N1 splash com emissionStart (0 → 80,78 m³);
  N2 emissores resolvidos de p.nodes e erro se faltarem; N3 medido (12–67 passos por
  quadro, 1059 em 31) → slot "near" (6 por quadro, 188 em 31; 0,647 → 0,149 s, bits
  idênticos; anel daria 3); N4 um registo por passo, Exchange e Reaction mortos saem; N7
  ends movidos, orçamento +40/+48/+8 B por célula (o "Scratch fora do orçamento" não
  reproduziu); N9 coluna repetida num lift é erro; N8 teto depth/cell = MAX_WINDOW; N6
  casco só para corpos dinâmicos; N10 waterline da esfera / steep (calota funda inclinada
  ainda difere até 16%, dito); N11 doc do trabalho 9/25 por célula e subpasso; N5
  helpers únicos (composition_clock, canonical_step em sr-sim, apply_oceans, WaterAttrs).
  Erro meu: li um `merge-tree --write-tree` inexistente como conflito; o merge-tree de
  três argumentos dá zero marcadores. Fundo em main depois da verificação de 2b2a1f8.
- **`sr_sim::gr` entregue** (Netuno, `feat/gr-schwarzschild` 5e33e9d, 3 commits TDD):
  RK4 em φ com captura/escape/cruzamentos, `impact_parameter`, `redshift`, versão f32
  com a mesma ordem de operações; oráculos: b_c = √27 M (erro 2e-11 a passo 0,01),
  horizonte, esfera de fótons, ISCO, Ω, deflexão exata por Romberg a 1e-14 (integrada vs
  exata 1e-8 em b = 10/20/100 M), 4M/b e série de 4ª ordem, Luminet = fórmula do shader a
  1e-15, T(r) de Shakura–Sunyaev; simetria ao bit; 10/10 testes; sr-sim 363. Limites:
  passo fixo 0,02 (corrigido por Netuno: φ_∞ medido a ≤ 1,3e-7 da quadratura para b de 5,3 a 100 M; o ~1e-3 era o limite frouxo do teste, não medida); só plano da órbita; redshift válido r > 3M; oráculo da
  deflexão perde precisão perto de b_c. Pedido a seguir: traçador de imagem CPU f64 de
  referência com o mesmo modelo de câmera do shader (sombra em pixels, simetria, g no
  azimute 0, comparação pixel a pixel). Mercurio já tem o shader em 415d811.
- **`gr::image` entregue** (Netuno, 28799fd): traçador CPU f64 por pixel com a convenção
  do shader copiada por escrito (raio do pixel, plano do raio, b, φ0, 4 cruzamentos, ψ, g,
  fundo); classes Captured/Background/Disk{r, ψ, g, ordem}; sem rayon, bits iguais.
  Medidas: sombra 12,915 px contra 12,764 esperados (|Δ| 0,15 px) a 256×160, câmera
  equatorial a 40 M, focal 100 px; simetria esquerda/direita a 1e-9; g = √(1−3M/r) na
  coluna central a 1e-12; disco visto de cima com ordens ≥ 1 entre a sombra e o disco.
  Exemplo `blackhole_cpu` escreve PPM. Falta o teste de Mercurio shader vs referência.
  Pedido: subsecção de física do SREP e entrada do ledger.
- Integração em `5e678b6`: merge de `fix/ocean-review` (N1–N11) em main local, sem
  conflito; fmt e hygiene limpos; verificação completa com GPU em
  `scratchpad/vrun_5e678b6.log` (CPU de `2b2a1f8` antes: sr-sim 353, sr-eval 396,
  sr-model 101, sr-3d 82, varreduras 38, clippy, fmt, hygiene limpos).
- SREP e ledger do buraco negro (Netuno, `feat/gr-docs` 12a7666 sobre o esquema de
  Saturno f7b6a30): parágrafo da implementação de referência, convenções, fórmulas com
  referências "citadas de memória, não lidas", medidas com commits e data; ledger com
  erros por passo, f32 vs f64 ≤ 9,4e-7 no ângulo, 2,6 s por imagem 1280×720 numa core.
  Teste apertado a 1e-6 (c7fcc13). A comparação shader–referência fica por preencher.
- **Câmera geodésica entregue** (Mercurio, `feat/blackhole-gr` e4b1f85 sobre o esquema
  de Saturno b385cb8 + gr de Netuno): shader `geodesic.wgsl` + `geodesic.rs`, padrão do
  disco com rotação kepleriana e semente, `camera@geodesics` no motor (erro se há outro
  objeto 3D, aviso se o buraco está sob câmera comum). Medidas: sombra 39,99 px para 40
  (√27 M, observador a 1e4 M); disco de frente simétrico e g = √(1−3M/r) a 2e-4; campo
  fraco g = 1/(1+v·d) a 4e-3 com o lado que se aproxima azul (mutações reprovam); imagem
  secundária a 75° (210 px acima, 1408 abaixo da sombra); contra a referência CPU f64 a
  96×60: 5760/5760 na mesma classe, pior erro 1,2e-5 (ângulo), 8,1e-6 (r), 3,1e-6 (g);
  contra gr::f32 2528/3186 raios a 1e-6, pior 1,2e-5 (contração do compilador da GPU);
  sem NaN; faixas == inteiro ao bit; semente determinística. Custo NVIDIA: 320×200 1,0 ms
  a 16 spp; 1280×800 4,3 ms a 16 spp, 17 ms a 64 spp (folga ~50× sobre o orçamento de
  1 s). Falta: render de 30 s (HD e pixel art) e a varredura completa do sr-gpu.
- SREP/ledger do buraco negro com os números do shader (Netuno, `feat/gr-docs` 2604773,
  "relatados pelo autor do teste, não repetidos"). Medição do N3 (cena da bola, 61 quadros,
  3 corridas, load1 15–18): solver 76–78% do custo do oceano em 1ª ordem e 88–89% em 2ª →
  pela regra, o anel: `fix/ocean-ring` (sobre 5e678b6, a verificar): últimos ≤16 passos
  guardados dentro de um quarto de checkpoint_bytes; 4 ofertas por quadro (3 passos + a
  re-amostra) em vez de 6; oceano 0,48–0,54 → 0,20–0,25 s (1ª) e 1,01–1,15 → 0,46–0,67 s
  (2ª), ~2× com carga a variar; bits idênticos; teste novo em sr-sim. Entregue:
  `fix/ocean-ring` 0b86cb7 (até 16 estados num quarto de checkpoint_bytes; a 720×720 cabe
  1); sr-sim 360, sr-eval 403; ledger em 3dc0188 (antes/depois por ordem, loads 18 e 15,
  variação até 15%, regra dos 60%, instrumentação fora do repositório). Branches prontos:
  `fix/ocean-ring` 3dc0188, `feat/gr-schwarzschild` c7fcc13, `feat/gr-docs` 2604773 (sobre
  f7b6a30 de Saturno).
- **Renders do buraco negro físico entregues** (Mercurio, `blackhole-gr/`):
  `blackhole-gr-30s.mp4` (1280×800, 24 fps, 720 quadros, câmera fixa a 60 M, 75° do eixo,
  disco 6–18 M, 64 raios por pixel, 16 ms de GPU por quadro) e
  `blackhole-gr-pixelart-30s.mp4` (320×200 ×4, 16 raios, LUT de 16 cores, legendas em 3
  tempos); conferidos por mim: ffprobe 720/30,000 s nos dois; MANIFEST 1452/1452 sha256;
  quadros extraídos do mp4 vistos: disco à frente, lado de trás arqueado por cima e por
  baixo da sombra, anel de fótons no bordo, lado que se aproxima mais brilhante. Look
  ajustado à mão declarado no MANIFEST (pico 4000 K, intensidade ×5, contraste 0,6,
  timeScale 4). Varredura completa do sr-gpu/CLI na branch em curso antes da integração.
- **Esquema do buraco negro entregue** (Saturno, `feat/blackhole-gr-schema` b385cb8 +
  f7b6a30, TDD com o corpus vermelho primeiro): XSD 1.3 `blackHole`, `accretionDisk`
  (innerRadius, outerRadius, temperatureScale, seed, angularPattern, contrast, intensity,
  timeScale, rotação), `camera@geodesics`; BH1–BH8 (versão; um buraco; disco nomeia
  buraco; ISCO; geodésica exige buraco; sem outros objetos; câmera > 3M; uma câmera
  geodésica) em Schematron e Rust; W03–W05; modelo tipado; oráculo a 326 documentos; um
  O(n²) apanhado pelo teste de desempenho e corrigido. SREP: secção com as fórmulas e
  oráculos calculados por Gauss–Legendre (ψ_sh, deflexão em 5 valores de b, g a 6M, séries)
  que batem com a referência de Netuno; "derivadas no texto, não conferidas nos artigos".
  S14: causa achada (dt = 1/24 + 3e-17 → fim do passo 0,2500000000000001 > quadro 0,25,
  idade −1e-16 recusada em `Particle::transform`), correção de uma linha em
  `Emitter::at`, a commitar com o teste; S15 a testar com o ring.obj de Mercurio.
- S14/S15 (Saturno, `fix/particle-frame-time` sobre 5e678b6): S14 8594d2e, correção de
  uma linha em `Emitter::at` (frame.time = max(pedido, estado)) com o teste que aplica o
  predicado do renderizador a todas as partículas de 100 quadros para 6 passos; bits
  iguais fora do caso (sr-sim 359, sr-eval 408, hashes de referência). S15 eef3628 não
  reproduz: o anel plano e o anel de 0,3 de Mercurio emitem no avaliador (>100 partículas);
  malha sem área dá erro; 4 testes de contrato ficam; o que Mercurio viu fica para ele
  verificar com --stats no render.
- **2026-10-06: o usuário aprovou todas as entregas** (pacote de sign-off da Fase 2,
  impacto em pixel art, buraco negro encenado em pixel art, buraco negro físico em HD e
  pixel art). `APPROVAL.txt` escrito em cada pasta de entrega. Segue o fecho do ciclo:
  varredura de Mercurio, merge único, verificação, merge com `origin/main`, push.
- **Fase 3, notas de desenho de Saturno** (scratchpad dele, `phase3-notes.md`, só leitura
  de código): fratura por contato: hoje só por tempo (`fracture@at` → `Fracture3`,
  `apply_fractures`); o registro de contatos já dá impulso, ponto e normal; contrato
  proposto `fracture@source`, minImpulse (2× peso por passo), energyFraction 0,3 (valor do
  motor), dispara no passo seguinte ao impacto, radial sem momento líquido, teto de energia;
  achado a verificar: o radial atual dá o mesmo módulo a toda peça, Σmv ≠ 0 em partições
  assimétricas. Escala de tempo uniforme: a limitação é `composition_clock` em
  `Group::detect`; proposta `uniform_clock` com a mesma afim para oceano, corpos e
  leitores, mundo rígido em t_w = m(T); a=1,b=0 = bits iguais. **Decisões:** ordem (1)
  teste/correção do momento da fratura existente, (2) fratura por contato com o contrato
  proposto, FRX5–7, corpus, SREP, (3) escala uniforme mínima; branch
  `feat/fracture-contact`; TDD. A Fase 3 começa por aqui; extensões do buraco negro só a
  pedido do usuário.
- **Verificação completa de `5e678b6`** (main com S1–S13 + N1–N11): sr-sim 359, sr-eval
  403, sr-3d 82, varreduras 38/38, sr-gpu na NVIDIA 446 e 1 falha, CLI 74 e 1 falha
  (`a_flat_document_still_renders_on_opengl`, ambiente GL, conhecida), sonda hero
  idêntica, fmt, hygiene e marcadores limpos. sr-model 100 e a falha do teste de tempo sob
  carga (passa 3/3 isolado). A falha nova é
  `grid_lighting_matches_the_exact_march_for_every_light_type` (PSNR grade vs marcha
  exata), que passa isolada; sr-gpu e sr-volume não mudaram desde 0bc940c (só o solver
  de fumaça, com hashes de referência iguais): intermitente: 10/10 isolado e 5/5 com o binário inteiro em paralelo passam
  (1 falha em ~16 execuções, sem mensagem capturada; o script de verificação passa a
  guardar o `panicked at` dos testes de GPU).
- **Integração em `e19e541`** (main local): merges, por ordem, de `fix/render-review`
  (M6), `fix/ocean-ring`, `fix/particle-frame-time`, `feat/gr-schwarzschild`,
  `feat/blackhole-gr-schema`, `feat/gr-docs` (um conflito no ledger, duas entradas no fim
  da lista, mantidas as duas) e `feat/blackhole-gr`; JSON válido (36 marcos), fmt e
  hygiene limpos. Verificação completa com GPU em `scratchpad/vrun_e19e541.log` (script
  agora guarda o `panicked at` dos testes de GPU). Depois: merge com `origin/main` (35
  commits de outros autores), nova verificação, push; cópia do SREP no sr-core.
- Varredura de Mercurio em `feat/blackhole-gr` (e4b1f85): fmt, clippy, hygiene e as 15
  mensagens limpos; sonda hero 720p idêntica; 49 binários na NVIDIA, 531 passam, 1 falha
  (OpenGL, ambiente). M11 (f280834, fumaça iluminada e atenuada pelos caminhos refratados)
  já está em main via fix/render-review. S15 corrigido na leitura: o emissor por malha
  emite e conta (--stats iguais nas três variantes) mas as partículas não aparecem onde
  está a malha (465 pixels quentes contra 6031 com a esfera): hipótese de posições erradas,
  repro em `pixelart-blackhole/scenes/repro-mesh-emitter/`; vai para Saturno com teste
  pelo avaliador (posições contra a superfície da malha).
- Fratura por contato (Saturno, `feat/fracture-contact` sobre 5e678b6): a7b6a0e momento da
  impulsão radial, 984e278 sim, 0702ec4 esquema FRX5–7 + corpus (316, oráculo de acordo),
  bb24ea6 eval + cache, 2c36ac0 cena `impact-block` + 4 testes de aceitação, 37d77ff SREP;
  sr-sim 365; resto a correr. Decisão: o emitterMesh (posições) vai antes da escala de
  tempo uniforme.
- **Fase 3, notas de desenho de Netuno** (`phase3-notes.md` no scratchpad dele, medidas):
  colisor da cratera: `trimesh_with_flags` = 93–95% do passo rígido que reconstrói (38–42
  ms: BVH 16 + topologia/pseudo-normais 25); o driver ainda calcula a cratera a cada passo
  depois de ela parar de crescer (2 ms/passo, 5–15% do quadro); `set_vertices` 17 ms;
  atualização parcial exigiria fork do parry. Referência física: caso 1, hump gaussiano
  em água rasa linear com solução exata por Hankel, erro L∞/A por σ/Δx = 4..32: 1ª ordem
  6,9e-2 → 1,4e-2 (ordem 0,63–0,88), 2ª 1,0e-2 → 3,9e-4 (ordem 1,3–1,7); caso 2, com
  dispersão real a frente é 3% mais baixa a σ/h=4 e a onda errada a σ/h=0,8: a onda do
  motor é propriedade do modelo, não previsão de água real; caso 3 (Ward & Asphaug) só
  depois de ler o artigo. **Decisões:** colisor A (revisão antes de calcular, grátis) →
  caso 1 como teste unitário → B (pré-construção paralela, bits iguais) → caso 2 como
  limite no SREP/ledger → C só com contatos iguais; D (fork) decisão separada; E recusado.
- S15 resolvido (Saturno, 936e6e7): não é defeito; mesh assets entram a 100 unidades por
  metro, Y-up (`sr_3d::Y_UP_METRES`), o anel de raios 6–11 nasce a 600–1100 unidades, fora
  do quadro; partículas a ≤ 1e-6 da superfície importada com T·Rz·Ry·Rx·S (3 testes com as
  fixtures de Mercurio); receita scale 0,01; linha no SREP. Contrato mantido. Momento da
  fratura: defeito confirmado (−0,99, −2,68 antes; ~0 depois). `feat/fracture-contact`
  rebaseado em e19e541 (7989b29..936e6e7), contagens pós-rebase a correr.
- M8, M7, M10, M9 (Mercurio, `fix/render-review-2` sobre e19e541): M8 64a2ef0 cópia do
  shader removida (teste do texto caractere a caractere); M7 487cf35 força bruta no
  adaptador de software só com SR_BRUTE_FORCE (medido: 619 s os dois testes; nota: ao
  pular aparecem como "ok"); M10a 0eb0068 `build_light_grids` extraída (7 hashes iguais);
  M10b 0fc9463 `dielectric_crossing` (vermelho real: `inside` zerado por reflexão fez
  falhar dois testes de força bruta até guardar com `refracted`); M9 c5dc54f docs da grade
  com teste das linhas. Sonda hero inalterada. MANIFEST do buraco negro em pixel art
  corrigido (emitterMesh = unidade; 759 ficheiros). A integrar no próximo lote.
- **Fase 4, análise de Mercurio dos abertos de render** (medições de hoje nas cenas de
  aceitação): (1) pontos escuros no horizonte a 8 spp: variância, não viés (517 pixels
  escuros numa faixa de 160×50 sem denoise, 217 com, 15 a 64 spp); hipótese: escolha
  estocástica de Fresnel em incidência rasante; (2) cáusticas: o raio de sombra analítico
  dá 0,581 do seco contra 0,631 da força bruta em ondas íngremes por faltar o foco; dias
  de trabalho; (3) câmera sob a água: valor zero nas cenas atuais; (4) farSea
  transmissivo não é a causa dos pontos (171 contra 217 com opaco); (5) segunda interface
  só com gelo/vidro sobre água; (6) manchas no chão não reproduzem (0,145 níveis por
  bloco); (7) interpolação na grade: nenhuma cena usa grade. **Decisão:** A, pontos do
  horizonte por estratificação da escolha de Fresnel (divisão do caminho só se não chegar
  aos limites ≤ 100 / ≤ 30), oráculos de viés (1% contra 64 spp), variância e identidade
  dos seis quadros sem vidro; o quadro hero muda e o hash de referência muda com
  evidência. B (cáusticas pelo jacobiano) adiado até haver plano com fundo sob ondas.
- **Pontos do horizonte, causa medida por Mercurio** (`fix/horizon-specks`): não é o
  Fresnel do dielétrico (o farSea é opaco); é o amostrador especular: (1) normais
  sorteadas de D·cos levam a reflexão para baixo da superfície em visada rasante e a
  amostra morre; (2) p_spec com teto 0,95 dá 1 em 20 ao lobo difuso escuro. Correção:
  VNDF (Heitz 2018) com pdf G1·D/(4 n·v) e teto 0,999: 137 → 8 de 2400 pixels escuros a 8
  spp (só VNDF 53, só o teto 92). Oráculo: albedo direcional por quadratura contra a média
  por linha a 256 spp, 3 rugosidades × 7 ângulos, diferença máxima 0,4%; sem G1 reprova.
  Consequência: todos os quadros path traçados mudam (mesma média); os hashes de
  identidade e da sonda hero mudam. Aprovado com condições: oráculo permanente, pixels
  diferentes/máximo/média por quadro, hashes novos com evidência no SREP e ledger,
  pacote de sign-off não se refaz, varredura completa, custo igual.
- **Fratura por contato entregue** (Saturno, `feat/fracture-contact` 936e6e7 sobre
  e19e541, 7 commits, 27 ficheiros, +2587): momento da impulsão radial corrigido (média
  ponderada pela massa tirada de cada peça), `fracture@source` no sim e no avaliador com
  cache, FRX5 (fonte é outro object3D dinâmico), FRX6 (derivados proibidos com source),
  FRX7 (minImpulse/energyFraction só com source), corpus, cena `impact-block` com 4 testes
  de aceitação, SREP, testes do emissor por malha. Verificação pós-rebase: sr-sim 382,
  sr-eval 421, sr-model 106, clippy, fmt, corpus idêntico. Revisto por mim; a integrar com
  `fix/render-review-2` depois da verificação de e19e541. Nota de processo: o target dir
  partilhado entre worktrees pode deixar um rlib velho "fresco" (hash de crate igual);
  touch e rebuild.
- Verificação completa de `e19e541` limpa: sr-sim 376, sr-eval 409, sr-model 106, sr-3d
  82, varreduras 38, sr-gpu na NVIDIA 470/0, CLI 74 e a falha conhecida de OpenGL
  (ambiente), sonda hero idêntica, fmt/hygiene/marcadores limpos.
- **Integração em `c7dba89`**: merges de `feat/fracture-contact` (96ce006),
  `fix/render-review-2` (be17c68) e `origin/main` (88e5774, 39 commits de outros autores:
  CI, formatação e clippy sobre código integrado, AV1, geoLayer, etc.); um conflito em
  `changes.rs` onde o upstream fez a mesma correção do S13 com o sha do manifesto: ficou
  a versão do upstream mais a nossa deduplicação; testes de changes passam; fmt, hygiene,
  JSON (36) e corpus (347, idêntico) limpos. Verificação completa com GPU em
  `scratchpad/vrun_c7dba89.log`; depois, push.
- **Pontos do horizonte fundidos** (`fix/horizon-specks` 06d6596 → main `6699ec2`): VNDF +
  teto 0,999; oráculo de albedo por quadratura permanente; faixa do horizonte 517 → 5 sem
  denoise, 217 → 1 com, média a 0,2% de 64 spp; os oito quadros mudam de pixels (até
  839 k de 921 k, máximo 201 níveis na pluma, 22 nas secas) com médias a 0,04% e tempos
  iguais; hashes novos (hero 720p t=3,0 = 24c6ab56…b72e, era 953a7b38…) adotados como
  referência no meu script; SREP/ledger com a tabela a seguir (Mercurio). O commit traz
  também `geodesic: None` em dois testes do upstream que não compilavam em c7dba89 (a
  verificação de c7dba89 foi parada por isso). Verificação completa em
  `scratchpad/vrun_6699ec2.log`. Cáusticas registadas como limite conhecido (0,581 contra
  0,631 em ondas íngremes; jacobiano adiado até haver plano com fundo sob ondas).
- 2026-10-06: o usuário pediu a reconciliação do roadmap (este plano) com o SREP e a
  atribuição a nós ou a outros desenvolvedores; delegada a um agente de leitura.
- Texto do amostrador (Mercurio, `docs/horizon-specks-record` 7691783 → main `f81bb82`):
  subsecção "Specular sampling at grazing views" com o oráculo, a tabela dos oito quadros
  e a referência nova do hero (24c6ab56…b72e desde 06d6596); frase dos "mesmos bytes"
  datada a c6195de; cáusticas como limite; parágrafo do adaptador de software atualizado
  (força bruta só com SR_BRUTE_FORCE); marco novo no ledger (37). Verificação de 6699ec2
  em curso vale para f81bb82 (só texto acima).
- Texto da câmera sob a água (Mercurio, c28bb41 → main `349d371`): o limite antigo do
  SREP e das notas de release ("a câmera sob a água não vê o sol no piso", "os mesmos
  bytes sem material transmissivo") estava desatualizado desde M3 e o amostrador; corrigido.
  Nota de desenho registada: janela de Snell e reflexão total já funcionam pelo cruzamento
  dielétrico; falta teste da borda da janela (45–49°), sonda só reta para cima, sem
  espalhamento volumétrico na água, sem cáusticas vistas de baixo; oráculo proposto
  (pixel = L·η²·(1−R(θ)) dentro da janela, ~0 fora, borda a 48,6° ± 1°); não fazer até
  haver plano submerso.
- **Reconciliação roadmap × SREP × histórico (agente de leitura, 2026-10-06, main 349d371):**
  totais: 34 itens implementados por nós (+2 partilhados com a base), 2 anteriores à base
  (1.13 batching, 1.15 teto 1 M), 0 por outros desenvolvedores, 8 parciais, 26 abertos.
  Outros desenvolvedores: nenhum commit toca sr-sim, os shaders de volume/path tracer, o
  SREP ou o ledger; 8 commits no escopo adjacente (shadow catcher, unevenness, node/
  materialOverride/tracking do object3D, adaptador 3D, creation_lock, partilha de Arc nos
  bakes, AV1, CI) nenhum citado no SREP. Lacunas acionáveis: (a) `ocean@density` previsto
  e ausente (1000 fixo em group.rs:311,656; SREP l.2040); (b) ledger sem marco para
  crater@source, registo de contatos, grupo acoplado/SRPHYS04, fumaça por cratera,
  fracture@source, particles3D@gas, 1.16, 1.17, câmera geodésica (só o SREP os documenta);
  (c) SREP não documenta 0.1/0.2 (stats/sonda), 1.16 (salto/diretório), 1.17 (falhas como
  erro); (d) inventário "exato" do SREP e tabela do object3DType sem os atributos upstream
  (shadowCatcher, unevenness, node, materialOverride, tracking) e a contagem "169
  asserções" datada (hoje 237); (e) plano 2.3 ainda com os nomes propostos (`on="contact"`)
  em vez dos reais (`@crater`, `@source`, `heatFraction`, `energyFraction`); (f) "144 quadros
  a 720p estrito, 18,9 min" só no plano, sem registo no ledger; (g) 1.15 medição de 500 k
  sem teste. O item E.9 do relatório (três MAJOR da luz pela água sem commit) é falso:
  89f5ff4, 5bd81dc, ca2eefe estão em main (fix/render-review). Abertos por fase: F1 1.5,
  1.9, 1.11 (estacionado), 1.14; F2 rasto e pressão da fumaça; F3 tudo menos 3.3 parcial;
  F4 tudo menos 4.3 e 4.4 parcial; F5 tudo; §6 barragem, explosão, pluma MTT, fornalha.
- **2026-10-06, ordem do usuário: seguir no ritmo de implementação pelo roadmap e pelo
  combinado.** Filas atribuídas (nota de desenho antes dos itens grandes; TDD; bits iguais
  sem os atributos): Saturno: docs → escala de tempo uniforme → 1.9 domínio que acompanha a
  pluma → 3.2 onda de choque (Sedov–Taylor como oráculo) → 3.4 fratura por tensão. Netuno:
  colisor A/B/(C) → teste de Hankel → 2.2 `ocean@density` → 3.3 colapso e deposição da
  cratera (manto ∝ r⁻³, conservação de volume) → 3.6 detritos granulares (ângulo de
  repouso). Mercurio: docs → 4.4 espuma como albedo → 4.1 espalhamento múltiplo
  (`medium@scatterBounces`) → 1.14 BVH persistente. Depois: 3.1 líquido 3D local e 3.5
  canal de vapor (maiores), 1.5 voxelização por caixa, 4.2/4.5/4.6/4.7, Fase 5.
- Docs de render (Mercurio, `docs/render-reconciliation` 7d9b95d: SREP com stats/sonda,
  salto/diretório com tempos, falhas como erro; ledger a aplicar depois do de Saturno).
  **4.4 espuma como albedo, desenho aprovado** com premissas corrigidas por leitura do
  código: a espuma do solver são partículas (Foam/Spray) nos checkpoints, não um campo; a
  espuma padrão é opaca (só o spray é transmissivo). Fração por vértice na CPU a partir das
  partículas Foam (kernel (1−x²)², idade, busca por grade), `whitewater@foamMode`
  particles|albedo (padrão particles), `foamRadius` (padrão 1 célula: cobertura na
  resolução da malha, limite dito), `foamAlbedo` 0,9, `foamRoughness` 0,8; fração no alfa
  da cor de vértice; gancho no shader da água só com a bandeira; raio de sombra refratado
  incluído. Oráculos: f = 1 → albedo 0,9 por quadratura a 2%; f = 0 → sete quadros iguais.
- **2026-10-06, pedido do usuário: otimizar build e testes ("estrangulados").** Medido:
  testes compilados com o perfil release do produto (`lto = "thin"`, `codegen-units = 1`);
  207 binários de teste de integração (sr-sim 40, sr-eval 60, sr-gpu 64, sr-model 26,
  sr-3d 12, CLI 5), cada um a linkar o crate inteiro com LTO; 5 target dirs (69 GB) sem
  cache partilhada; cinco `cargo test` em paralelo (4 sessões × 4 jobs em 8 cores, carga
  13–19); sem sccache, mold ou lld e sem rede (crates.io 403). Feito: `[profile.ci]` em
  main (c7632a8; herda de release, sem LTO, 16 codegen units; binários em target/ci) para
  testes e verificações; fila `sr-build` em `~/.local/share/scene-render/bin/` (um build
  completo de cada vez, 8 cores a quem tem a vez; incrementais fora com 2 jobs); script de
  verificação com `--profile ci` e 8 jobs na fila. Atribuído: fundir os ficheiros de
  teste em poucos binários por crate (Saturno: sr-sim/eval/model/3d/CLI; Mercurio:
  sr-gpu), medindo tempo de `--no-run` do zero e tamanho do target antes/depois. A
  verificação completa passa a correr só no fecho de ciclo; os lotes verificam só os
  crates tocados.
- **Integração em `e9cfb07`**: tooling (`[profile.ci]` c7632a8, `SR_CARGO_PROFILE` nas
  varreduras ed96b08), `docs/ledger-reconciliation` (Saturno a26493f: 6 marcos novos, 43 no
  total; contagem de asserções corrigida: 228 no .sch, 77 cinemáticas, em fa63e5d 169/66;
  inventário declara o escopo e nomeia os atributos do upstream; correções no texto dele:
  hash pré-rebase e "1,5 s" → t = 3 s; ordem partículas→fumaça no texto do gas) e
  `feat/group-time-scale` (Saturno 4bed5cb: relógio afim único para oceano, corpos e
  leitores; bits idênticos entre grupo escalado em T e cena sem grupo em s·T para 5
  escalas, aninhados e timeOffset; erros que nomeiam os dois nós; limites: fumaça de
  cratera e partículas no oceano continuam no relógio da composição). A verificação com o
  perfil release de 6699ec2 foi parada (lenta, superada); verificação com o perfil `ci`
  pela fila `sr-build` em `scratchpad/vrun_e9cfb07.log`, que serve de medição do ganho.
- **Mais otimizações de build (pedido do usuário):** `sr-build` passa a dar aos builds na
  fila um target dir partilhado (`~/.local/share/scene-render/target-shared`; crates do
  crates.io compilam uma vez para todos os worktrees e perfis; artefatos de crates do
  workspace são por caminho e não colidem; `SR_BUILD_LOCAL_TARGET=1` desliga); o script
  de verificação usa o mesmo. `incremental = true` no perfil ci fica guardado em patch
  (`scratchpad/ci-incremental.patch`) para aplicar depois da verificação em curso: mudar o
  perfil a meio invalidaria os artefatos dos passos seguintes. Pendentes sem rede: sccache,
  mold/lld; gold existe no host, a medir num link antes de adotar.
- **Fusão dos testes do sr-gpu medida** (Mercurio, `test/merge-sr-gpu-tests` f419a17 sobre
  e9cfb07): 62 ficheiros → 6 binários por área (raster_effects 227 testes, scene_3d 137,
  path_tracer 45, volume_grids 28, adapters 37, process_env 2) + backends à parte; perfil
  ci, target vazio, na fila: 65 executáveis, 3,08 GB, 473 s → 8 executáveis, 0,44 GB,
  283 s (−40% tempo, −60% disco em binários; target 4,0 → 1,6 GB); 528 nomes iguais;
  NVIDIA: tudo passa menos os dois de OpenGL (ambiente). Achado: dois testes que se
  reexecutam por nome quebravam no módulo: um falhava e outro passava sem testar nada
  (o filho corria 0 testes); corrigidos com `exact_test_name` e asserção de que o filho
  correu 1 teste. Docs de render: ede72dd (SREP) + c0da295 (ledger, 47 marcos). A integrar
  depois da verificação de e9cfb07.
- **Colisor da cratera e referência física entregues** (Netuno, `perf/crater-collider`
  sobre e9cfb07, 6 commits): A 4b51e7d memo da lei por impacto (809 → 1 avaliação; quadro
  317–435 → 235–296 ms na terra, 107–119 → 52–55 ms no oceano, load1 18–23); B 862d88b
  malhas dos passos de um quadro construídas em paralelo (bits iguais 1/2/8 threads e
  replay; janela de contato: oceano 3,9–4,2 → 2,1–2,5 s, terra 4,2–4,4 → 3,7–4,0 s, load1
  12–13); C (set_vertices) tentado, mais lento que B, deixado com números no ledger; D
  (fork do parry) decisão separada. Caso 1 06a79df/c6afd99: teste `ocean_reference` com a
  solução exata de Hankel (1ª ordem 6,9e-2 → 2,6e-2 A; 2ª 1,0e-2 → 9,3e-4 A; ordens 0,70 e
  1,71; volume 3e-14; 15,8 s; passou à primeira, dito). Caso 2 07ee425: limite de
  dispersão no SREP/ledger com os números. A integrar após a verificação. Netuno segue
  para `ocean@density`.
- **`ocean@density` entregue** (Netuno, `feat/ocean-density` dda3ae8..7533c86 sobre
  e9cfb07): XSD default 1000, corpus (349, oráculo de acordo), empuxo/arrasto/reação,
  momento do respingo ÷ densidade, alvo da lei da cavidade; testes: calado a ≤ 0,03 m em
  1000/1030/1100; default ao bit. Honestidades no ledger: o modelo preenche o default
  (atributo nunca ausente); testes de eval escritos antes mas não corridos em vermelho.
  **3.3 notas** (phase3b-notes.md): o núcleo tem o bowl até à crista (1,285 V) e a borda
  0,326 V, o chão perde 0,959 V e os 0,8 V de ejetos não estão no chão. **Decisões:**
  orçamento de volume como contrato (teste que falha hoje) → manto analítico
  t0 (R_rim/r)³ truncado a 20 R_rim, partículas assentes removidas → manto por pontos de
  aterragem só como medição → sem fator de colapso (o raio da lei é o final; transiente
  fica como limite) → sweeps do mar refeitos. **3.6:** híbrido partículas→campo granular
  com ângulo de repouso; medir anisotropia do stencil (4 vs 8) no oráculo do cone antes.
- **Fusão dos testes dos outros crates medida** (Saturno, `perf/test-binaries-sim` b619674
  sobre e9cfb07, 138 renomeações, nenhum teste mudado): sr-sim 41 → 7 binários (21,9 →
  12,6 s; 120 → 31 MB), sr-eval 62 → 11 (97,2 → 32,5 s; 2158 → 375 MB), sr-model 27 → 5
  (38,0 → 30,8 s; 273 → 53 MB), sr-3d 13 → 3 (15,4 → 9,8 s), scene-render 6 → 2 (13,0 →
  7,4 s); executáveis de teste 2,65 → 0,52 GB; mesmas condições (target local, deps
  compiladas, `cargo clean -p` antes); nomes folha iguais (384/478/117/90/84). Em binário
  próprio: os 3 com global_allocator, `perf` (usado pelo ci.yml), hero_hires. Suíte na
  fila com perfil ci: sr-sim 382, sr-eval 466, sr-model 117, sr-3d 91, CLI 83 e a falha de
  OpenGL do ambiente (igual sem a mudança). Com o sr-gpu: 207 → 36 binários de teste.
- Verificação de e9cfb07 (perfil ci, parcial): sr-gpu na NVIDIA 522 passam, 6 falham,
  todos OpenGL (quatro `gl_*` do backends, dois de hardening), com pânico dentro do wgpu
  (wgpu_core.rs:1414) na criação do device GL: ambiente, como a CLI; o `backends` agora
  termina (não fica pendurado) e falha rápido. Item para Mercurio: testes de GL saltam
  limpo quando o device não pode ser criado (sonda em processo filho ou catch_unwind).
- **Verificação de `e9cfb07` com o perfil ci fechada** (21:41–22:54, com espera na fila
  atrás da medição de base de Saturno): sr-sim 382, sr-eval 466, sr-model 117, sr-3d 91,
  varreduras 38, sr-gpu 522 e 6 de OpenGL (ambiente), CLI 83 e 1 de OpenGL, sonda hero
  com o hash novo (24c6ab56…: o perfil ci não muda a imagem; os hashes de referência da
  fumaça passam, o perfil não muda bits dos solvers), fmt, hygiene e marcadores limpos.
  Integração dos cinco lotes pendentes em curso (docs de render, colisor, densidade,
  fusões de testes); depois o patch do incremental, verificação curta e push.
- **Integração em `7a2de93`**: merges de `docs/render-reconciliation` (5839646),
  `perf/crater-collider` (252e994, conflito no ledger resolvido com as duas entradas),
  `feat/ocean-density` (e897f96, idem), `perf/test-binaries-sim` (7dbf224),
  `test/merge-sr-gpu-tests` (d0f8b3a) e o patch do incremental (7a2de93); ledger com 51
  marcos; corpus 349 idêntico; fmt e hygiene limpos. Os testes novos de Netuno
  (`ocean_reference.rs`, `surface_prefetch.rs`, densidade) ficaram como binários próprios
  ao lado das áreas fundidas; a mover depois. Verificação (perfil ci, fila, 36 binários de
  teste) em `scratchpad/vrun_7a2de93.log`, lançada às 22:56: mede o ganho da fusão.
- **1.9 `pyro@follow`, nota de Saturno aprovada**: janela com o mesmo número de células
  que desloca em múltiplos inteiros de célula quando a fumaça chega a followMargin (12,
  valor do motor ligado ao W02) de uma face aberta, largando só fatias vazias (nunca
  massa; se a fatia de trás não está vazia, para de seguir esse lado e diz); `window` no
  State e nos checkpoints; turbulência por célula global; exportação com a transformada
  da grade a mover-se e o nó quieto; PYRO9 (follow só com boundary open); seis oráculos
  (bits iguais sem deslocar; cópia ao bit; pluma 3× a janela nunca cortada; réplica;
  exportação; custo ≤ 5% sem deslocar); alinhamento do reticulado da grade de luz fica
  com Mercurio, condicionado a volumes em movimento. Três eixos seguem.
- 1.9, achado de Saturno (passo 1, 4bfc4e9): num puff de 0,5 s numa coluna aberta a
  fumaça nunca deixa uma fatia de trás exatamente vazia (talo e cauda de interpolação:
  11% da densidade nas 56 linhas de trás ao passo 100), logo "só fatia vazia" quase nunca
  avança. **Decisão:** `followLoss` (0–1, default 0 = nunca largar massa), massa largada
  contada em `lost` no estado e no checkpoint, oráculo massa antes = depois + largada;
  regra dura: a janela nunca larga uma fatia com fonte ativa (seguir a cabeça largando a
  base separaria a pluma do chão; se não cabe, janela maior, não loss); para a hero medir
  a extensão da pluma contra a janela antes de escolher.
- 3.6 stencil medido em protótipo (Netuno): cone de 35°, 4 vizinhos 38,9–44,5° (pirâmide),
  8 vizinhos 35,1–36,9° (raspa os 2°), 16 vizinhos 35,1–35,5°: **16 vizinhos**. 3.3: a lei
  só fecha com bulking 1,126 (borda da lei 0,326 V + manto 0,8 V); **decisão:** borda da
  lei fica; contrato V_borda + V_manto = bulking × V com `crater@bulking` (1–1,3) default
  1,126 derivado da lei; `crater@mantle` default false. `feat/crater-mantle` começou
  (d804185 move os testes de Netuno para as áreas).
- **Verificação de `7a2de93` limpa em 22 min** (22:56–23:18, perfil ci, fila, 36 binários;
  antes 73 min com fila e 2 h sem): sr-sim 386, sr-eval 471, sr-model 117, sr-3d 91,
  varreduras 38, sr-gpu 522 e os 6 de OpenGL, CLI 83 e 1 de OpenGL (ambiente), sonda com o
  hash novo, fmt/hygiene/marcadores limpos. `origin/main` avançou 19 commits de outros
  autores desde o merge anterior: merge antes do push.
- Merge de `origin/main` (b8f69fd, 19 commits) conflita no XSD (2 regiões, 587 linhas) e
  no Schematron (1 região, 181 linhas): o upstream vendorizou o schema 1.3.0 do sr-core
  (cdf3630, b9dff58), com guard de SREPs pendentes e renomeação de R45 → R47; os nossos
  elementos cinemáticos têm de ser reconciliados. Abortei o meu merge; Saturno (dono do
  esquema) faz `merge/upstream-schema-1.3` sobre 7a2de93 com corpus, oráculo, sr-model e
  hygiene; eu integro, verifico e envio. pyro@follow em pausa até lá.
- **Merge do upstream resolvido por Saturno** (`merge/upstream-schema-1.3` db80223 = main):
  método por componente com a base 88e5774: 33 componentes do XSD e 4 patterns só do
  upstream (connector, pdf, pointListType, nodeCoreAttributes, shape width/height
  opcionais, regionPadding, report, fontPolicy, readingSpeed, R45 → R47), 7 componentes e
  3 patterns só nossos, blackHoleType/accretionDiskType e os patterns BH só nossos, um
  componente mudado pelos dois (nodeChoice: connector + blackHole + accretionDisk, os
  dois mantidos); nada nosso removido pelo upstream. Resultado = upstream + exatamente
  as nossas adições; 257 asserções com ids únicos (246 deles + 11 nossas). O guard de
  SREPs pendentes (`pending.rs`) é para atributos aceitos e não implementados (E22), não
  toca nas nossas. `schema/UPSTREAM` com a ressalva. Facto: o sr-core 1.3.0 já traz os
  tipos cinemáticos (oceanType, craterType, fractureType, pyroType, particles3DType,
  volumeAssetType, …): o rascunho entrou no esquema publicado. Verificação completa em
  `scratchpad/vrun_db80223.log` (23:28); push a seguir.
- **3.3 colapso e deposição, entregue** (Netuno, `feat/crater-mantle` sobre 7a2de93, 7
  commits, a rebasear em db80223): bowl escava exatamente V com a profundidade e o raio
  da lei; manto t0 (R/r)³ cortado a 20 R e renormalizado a 0,8 V; borda = bulking·V −
  0,8 V; `crater@mantle` (default false, bits iguais) e `crater@bulking` (default 1,126
  derivado; 1 e 1,3 testados); CRT10/CRT11; conservação a 1e-6 V; ejetos assentam
  (`Spec::settle`, 0,5 m/s como escolha) e saem da contagem (1000 → 621 vivos aos 5,5 s na
  terra); replay idêntico; sr-3d 95, sr-sim 389, sr-model 117, sr-eval 473. Limites:
  transiente não modelado; manto analítico; comparação com pontos de aterragem por fazer;
  testes de orçamento escritos antes mas não corridos em vermelho; sweeps do mar a refazer.
- **Erro meu, retirado:** o target dir partilhado da fila misturou crates do workspace de
  worktrees diferentes: o clippy de main em db80223 linkou o sr-sim de `feat/pyro-follow`
  ("missing field `follow` in initializer of sr_sim::pyro::Spec"); a minha afirmação de
  que os artefatos são por caminho estava errada. `sr-build` volta ao target local;
  agentes avisados para repetir no local o que correu no partilhado (21:40–00:05). A
  verificação de db80223 passou todos os testes antes da contaminação (sr-sim 386,
  sr-eval 476, sr-model 117, sr-3d 95, varreduras 38, sr-gpu 522/6 GL, CLI 83/1 GL, sonda
  igual), mas o clippy falhou por ela: repetir a verificação no target local antes do push.
- Item 3 do 3.3 medido (Netuno, 18b8595): 84–86% da massa lançada cai dentro da crista
  (volta à cratera), o resto entre 1 e ~3,5 raios com declive log-log −4 a −7, mais íngreme
  que o cubo; as partículas não reproduzem o manto analítico, que fica como afirmação da
  lei (limites: chão plano sem deformação, sem arrasto). 3.6 núcleo (ea4a5c3):
  `sr_sim::granular::Bed`, relaxação determinística de ordem fixa sobre 16 vizinhos, cone a
  35° em 34,1–35,6° (10 células), volume a 1e-15; montes pequenos (≤ 8 células) fora dos
  2°, dito. Ligação à cena aprovada: `debris@repose` (default off), exclusivo com
  `crater@mantle`, leito alimentado por absorbed() com estado por instante e checkpoints,
  depósito como elevação na deformação da cratera. **Integração em `8508fa6`** (merge de
  `feat/crater-mantle` ea4a5c3 sobre db80223); verificação no target local em
  `scratchpad/vrun_8508fa6.log` (00:00); push a seguir, com o merge do commit a mais do
  upstream (3394a07) se ainda estiver lá.
- **3.6 ligado à cena** (Netuno, `feat/granular-bed` 1c407be sobre 8508fa6): `crater@repose`
  (10–60°; CRT12 exclusivo com `mantle`, CRT10 exige `source`); partículas em repouso
  saem do emissor e registam posição e volume num ExchangeLog dentro de splash::Log
  (comparado ao bit no replay); `debris::State` verte os volumes por passo num
  `granular::Bed` (144×144, 12 células por raio de crista) sobre a cratera crescida, com
  checkpoints; o leito entra no kernel da cratera como `Deposit` (altura + jacobiano).
  Teste vermelho primeiro; 1000 ejetos: 169 634 kg lançados, 73 621 kg assentes aos 5,5 s,
  depósito 35,0577 m³ = massa/2100 a 1e-9; replay ao bit; custo no ruído. Limites: partículas
  e mundo rígido aterram sem o depósito; render na GPU não verificado; relaxação sobre a
  cratera final; além de 6 raios vai para a borda do leito; normais descontínuas.
  sr-3d 98, sr-sim 395, sr-model 117, sr-eval 480, corpus 359. Correção de Netuno: 1c407be
  tinha 4 ficheiros do corpus por commitar (lidos da árvore de trabalho); a ponta a integrar é
  232ae1c. A integrar após o push.
- **Fase V, nota de Netuno (V.4/V.5) e decisões:** colisor = forma de voxels nativa do
  rapier 0.36/parry 0.31 (`ColliderBuilder::voxels`, `set_voxel`, contatos voxels×malha,
  `mass_properties_voxels`); caixas gregas só para terreno estático; momentos inteiros em
  ordem fixa para massa/centro/inércia (`rigidBody@density` obrigatório, `mass` junto é
  erro); fragmentos por SLOTS pré-criados (`maxFragments` 64, `fragmentMinCells`,
  `fragmentOverflow=error|dust`) como as peças da fratura; ejetos das células removidas
  com a lei mapeada por quantis; estado = ocupação base + registo de edições, revisão =
  hash; componentes por 6-conectividade em ordem de varrimento; `fracture@partition`;
  mantle/bulking/repose recusados em donos de voxels. Tipo partilhado `Occupancy` em
  sr-3d, definido por Netuno (P1). Sete oráculos vermelhos primeiro; ordem P1 ocupação +
  momentos + componentes → P2 colisor rapier + repouso + sonda de custo 1e4..1e6 →
  P3 slots → P4 remoção pela cratera + ejetos por célula → P5 fratura → P6 srvseq +
  cache por revisão. Branch `feat/voxel-physics` sobre 8508fa6, já em curso.
- **1.9 `pyro@follow` entregue** (Saturno, `feat/pyro-follow` b6c65ca sobre db80223, 6
  commits; sr-sim 400, sr-model 119, sr-eval 480; 9 hashes de referência iguais; PYRO9–11,
  corpus 353). Achados: loss 0 nunca segue (cauda numérica nunca é zero; talo de 11%):
  medido num blob (ref. 164,3): fixa 12,9, loss 0 12,9, 1e-9 42,6, 1e-6 174,0, 1e-3 181,0;
  política simétrica movia a janela para trás na hero (bola de fogo expande para baixo):
  corrigida para pedir espaço só na face para onde a fumaça vai (212fd3f); hero (64×52×64
  células de 3): sem follow 98 de 2972 de fumaça no topo a 6 s; pluma real de 145 unidades
  só cabe com margem 3; `follow margin 3 loss 1e-3` → 0,28 no topo, total 2977 (contra 3016
  no domínio alto); alternativa janela 64×80×64 (+54% custo). **Decisões:** medir já o
  custo da passada da decisão (limite 5%; senão só fatias de borda); hero recebe a opção
  (a) no ficheiro de exemplo, com a sonda a ser remedida por Mercurio; loss default 0 fica,
  dito que seguir exige loss > 0.
- **Verificação de `8508fa6` limpa no target local** (00:00–00:30; clippy limpo desta vez):
  sr-sim 395, sr-eval 478, sr-model 117, sr-3d 95, varreduras 38, sr-gpu 522 e 6 GL, CLI
  83 e 1 GL, sonda igual, fmt/hygiene/marcadores limpos. Merge do commit a mais do
  upstream (3394a07, regra de caixa das formas) → `de68d7e`; corpus 355 idêntico,
  sr-model 120, clippy 0. **Push de `main` para `origin/main`** (de68d7e), 2026-10-07 ~00:40.
  Próximo ciclo: `feat/granular-bed` 232ae1c e `feat/pyro-follow` (após rebase e medição
  do custo da decisão), render da hero com follow por Mercurio.
- **Fase V, P1 e P2 entregues** (Netuno, `feat/voxel-physics` 8e9c1b0 sobre 963b614):
  `sr_3d::occupancy` (grade esparsa, bricks de 8³, revisão só em edição real,
  changed_bricks_since, fingerprint independente da história, momentos exatos em i128 →
  massa/centro/inércia iguais ao bit em qualquer ordem, componentes por 6-conectividade com
  oráculos à mão); `Shape3::Voxels` = colisor de voxels do rapier, propriedades de massa
  iguais às exatas a 1e-12; repouso de um bloco de células contra a caixa igual a 8,6e-7 m a
  1/60 s (margem 14%) e 2e-8 a 1/240; pilha de três blocos a 2,4 mm a 1/60 (a própria pilha
  de caixas deriva 4 mm); sonda de custo: mundo 2,3/18/166 ms para 1e4/1e5/1e6 células,
  passo 0,013–2,6 ms, checkpoints 0,2–0,4 MB; sem caso para o fallback. Não medido: custo
  de editar células com checkpoint a segurar a forma (decide P4), jitter em repouso,
  voxels sobre a malha da cratera. Pedido: construtor em bloco e paleta RGBA no Occupancy
  para o importador e o meshing.
- Fase V, custo de edição medido (Netuno, 33e1a07): remover células de um colisor de
  voxels com checkpoint a segurar a forma = cópia + edições: 1 célula 0,012/0,12/0,94 ms a
  1e4/1e5/1e6 células; 100 000 células de 1e6 em 7,2 ms (cópia ~1 ms); cópia privada de
  1e6 células 1,8 MiB: a remoção pela cratera custa milissegundos; a forma editada tem de
  entrar no orçamento do checkpoint (~1,8 B por célula). `Occupancy::from_bricks` /
  `from_cells` (duplicados e índice 0 são erros; fingerprint igual à inserção célula a
  célula) e paleta RGBA de 256 como campo, com `palette_revision` e
  `appearance_fingerprint` separados da revisão geométrica. P3 a seguir; a conversão
  Occupancy → corpo fica como função pura até o esquema de Saturno existir.
- `pyro@follow` final (Saturno, 8a49964 sobre 963b614, 8 commits): custo da decisão
  0,0093 s contra 0,423 s de passo a 128×104×128 (2,2%) e 0,0334 contra 1,407 s a
  192×156×192 (2,4%), dentro dos 5%; hero com `follow="true" followMargin="3"
  followLoss="0.001"`; SREP com os limites (margem default não cabe; loss > 0 necessário;
  margem em células; hero-hires precisaria de 6); contagem de asserções 263 (83
  cinemáticas, 17 que o sr-core 1.3.0 não tem). sr-sim 409, sr-model 122, sr-eval 484. A
  integrar com 33e1a07 após a verificação de 963b614; render da hero (quadro 120 e sonda)
  por Mercurio para o marco do ledger.
- **Fase V, nota V.1 de Saturno aprovada** (`design-voxels.md` no scratchpad dele):
  `<voxelAsset id src format=vox|srvol sha256 model fromMesh cellSize maxCells
  maxMemoryMiB voxelGrid>` (um de src/fromMesh; limites checados antes de alocar; modelo
  ou cena inteira do `.vox` com grafo de cena no quadro 0; eixos MagicaVoxel → cena
  (x, −z, y)); produz `Occupancy` + cores RGBA + propriedades MATL mapeadas; parser `.vox`
  em sr-3d (só std, cada chunk checado), conversão srvol em sr-3d (aresta nova
  sr-3d → sr-volume), fromMesh em sr-eval com o teste de ponto dos colisores;
  `<object3D primitive="voxels" voxels cellSize palette=file|IDREFS surface=blocks>`;
  VOX1–VOX7 com corpus; oráculos: fixtures `.vox` por script independente (modelos,
  grafo, MATL, sem RGBA, v150/v200, truncados e corrompidos em fuzz), ida e volta srvol,
  fromMesh contra contagens analíticas e força bruta, determinismo. Decisões: nomes
  aceitos; importador em sr-3d; `palette="file"` default com RGBA, sem RGBA exige
  `palette`; índice u8; sem `.vox` real na máquina nem rede: três factos ficam "não
  provados contra o programa real" até o usuário trazer um ficheiro. Estimativa 4,5 dias.
  Branch `feat/voxel-asset`.
- **3.2 onda de choque, nota de Saturno aprovada:** o solver de fumaça é incompressível
  (sem equação de estado nem velocidade do som); a fase de Sedov–Taylor da rocha autoral
  (E = 4,5e8 J) chega a Mach 1 em ~0,01 s, dentro do primeiro quadro: o que se vê é o
  efeito sobre a poeira, não a frente. Substituto declarado: pistão esférico de raio R(t) =
  ξ(γ)(E t²/ρ0)^(1/5) com divergência-alvo 3Ṙ/R, que a projeção transforma no escoamento
  exterior u = ṘR²/r² (solução exata de potencial); energia cinética exterior 2πρṘ²R³,
  constante no tempo para R ∝ t^(2/5), calibrada por `kineticFraction` calculada da
  solução de semelhança; ξ(γ) por quadratura (oráculo `sedov` em sr-sim). Limites: ação à
  distância, sem salto nem aquecimento, R_max = 0,3·(E/p0)^(1/3) (valor do motor), forma
  pobre a dt grosso (medir a 1/240), corpos e oceano não recebem a carga. Elemento
  `pyroBlast` (crater, kineticFraction, gamma, ambientPressure, ambientDensity), PYC5–7.
  Solver compressível fora desta fase. Ordem de Saturno: V.1 → 3.2 → 3.4.
- **3.4 fratura por tensão, nota de Saturno aprovada:** peças ligadas por juntas Weld do
  rapier (já existem no World3, com break_force linear da última subpassada) com
  `break_force = σ_c × A_ij` e termo de flexão, lido do impulso acumulado no passo; quebra
  progressiva, peças passam a colidir; `fracture@mode=impact|stress`, `strength` (Pa)
  obrigatório sem default; sem empurrão por energia (só o gradiente de velocidade do
  impacto, dito). Risco principal: rede de Welds do PGS a 256 peças e 100 m/s pode tremer:
  espigão primeiro, critério fixado (deriva < 1 mm em 2 s, impulsos sem crescimento, sem
  NaN, energia não cresce, momento a 1e-9, ≤ 5 ms por passo a 256 peças), senão rede de
  molas. O kernel não regista interfaces: tipo partilhado `PieceGraph` em sr-3d
  (interfaces com área, normal, centróide; T-junctions e remendos múltiplos), usado
  também pela partição de voxels. Oráculos (a)–(g). Estimativa 8–10 dias. Ordem: V.1 →
  3.2 → 3.4.
- Verificação de `963b614` limpa (01:06; sr-sim 395, sr-eval 480, sr-model 120, sr-3d 98,
  varreduras 38, sr-gpu 522/6 GL, CLI 83/1 GL, sonda igual, clippy, fmt, hygiene).
  **Push `963b614`** a origin. **Integração em `d87fc65`**: `feat/pyro-follow` (8a49964) e
  voxels P1–P2 (33e1a07); corpus 363 idêntico, 56 marcos, fmt e hygiene limpos;
  verificação em `scratchpad/vrun_d87fc65.log`. Mercurio renderiza a hero com e sem follow
  (sonda t=3,0; quadros 120 e 143; pixels de fumaça no topo) para o marco do ledger, em
  `/home/pals/renders/cinematic-impact/phase3/hero-follow/`.
- **Fase V, P3 entregue** (Netuno, c41bbbf): `World3::with_voxel_splits` com slots
  desligados criados no build (contagem de corpos fixa, replays como as peças da fratura);
  `Driver3::voxel_cut` como função pura validada antes de tocar no mundo (célula
  inexistente, massa não positiva, mais peças que slots = erros); pai perde células,
  toma a massa nova e a velocidade do novo centro; peças tomam pose, spin e v + ω×(c−c_old);
  oráculos a 1e-12 e momento a 2e-12; replays de checkpoint ao bit; formas de voxels
  cobradas nos checkpoints por célula (~2 B). sr-eval puro, não ligado: `voxels::body` e
  `voxels::cut` (componentes, quem fica Largest|Anchored, min_cells → pó, overflow erro ou
  pó). Limites: segundo corte e slot como pai não testados; contatos aresta/canto da peça
  com o pai no primeiro passo não medidos. Ordem: P4 parte pura (com teste do segundo
  corte) → P5 sobre o `PieceGraph`.
- **Verificação de `d87fc65` limpa** (01:07–01:28): sr-sim 411, sr-eval 485, sr-model 122,
  sr-3d 111, varreduras 38, sr-gpu 522/6 GL, CLI 83/1 GL, clippy, fmt, hygiene. **A sonda
  hero 720p t=3,0 mudou** de 24c6ab56… para eeb71e14…: esperado, porque `hero.scene.xml`
  passou a ter `follow="true" followMargin="3" followLoss="0.001"` (8a49964); a causa
  exata (janela já deslocada a 3 s, ou o hash da turbulência por célula do espaço com
  follow ligado) e a magnitude (pixels, máximo) vêm da medição de Mercurio antes de a
  referência ser adotada. **Push `d87fc65`** a origin. Merge de P3 (c41bbbf) em seguida.
- **`.vox` reais obtidos** (09:12): a máquina alcança o GitHub (só o crates.io está bloqueado). 13 ficheiros em `/home/pals/assets/vox-samples/` (README com fontes, licenças MIT, inventário de chunks, sha256): 7 de ephtracy/voxel-model (v150, SIZE/XYZI com e sem RGBA, até 126 células de lado), 5 de dust-engine/dot_vox (v150 com grafo de cena nTRN/nGRP/nSHP, LAYR, 256 MATL, chunks de render; axes.vox v200 com 4 modelos), 1 negativo (magic DOOD). Entregues a Saturno para os três factos do formato e como fixtures (os pequenos, com a licença ao lado). Sessões reiniciadas ~09:05; o meu nome passou a rs-scene-render-a9 [2c80ed]; a verificação de c075d1f sobreviveu ao reinício.
- **Hash da hero atribuído (Saturno, 09:13), com um defeito**: (a) a janela da hero desce 2 células (y=-78 → -72) entre 1 e 2 s porque a bola de fogo em expansão e a fumaça sem direção em y pedem margem na face de baixo (política, não defeito; previsão "só perto de 5 s" estava errada); (b) **defeito contra o oráculo 1**: com follow ligado a deslocamento zero o ruído de turbulência era chaveado pela célula global, logo bits diferentes de follow ausente (o teste anterior usava turbulência 0 e não apanhou). Correção em `fix/follow-zero-window` sobre c075d1f: célula dentro da caixa original = índice linear, fora = chave própria (bit alto + 21 bits por eixo); teste vermelho `…_with_seeded_turbulence_too` agora verde. Decisões: correção aprovada; política de margem muda em segundo commit (uma face só pede espaço quando a velocidade do ar ao longo da fumaça aponta para ela), com previsão registada antes de medir: se a janela ficar em -78 até 3 s, a sonda t=3,0 volta a 24c6ab56…. Os quadros de Mercurio em d87fc65 ficam como "antes da correção"; a referência é medida no commit corrigido. Sessões: Saturno = rs-scene-render-47, Mercurio = rs-scene-render-20, Netuno = rs-scene-render-ee.
- **P3 de voxels NÃO está fechado (Netuno, 09:16)**: revisão adversarial própria (53 agentes) confirmou 18 achados; dois defeitos reais no P3 já em c075d1f: (a) `frame_at` não chama `apply_voxel_cuts` enquanto `step_once` chama, logo o quadro do passo do corte depende de vir do log ou de uma chamada fresca (dependência de ordem; os testes de replay pediam tempos tardios primeiro e os orçamentos de checkpoint de 4–6 kB não admitiam checkpoint nenhum, logo não repetiam nada); (b) corte que deixa parent_mass 0 desativa o pai mas `sync_visibility` reativa-o no passo seguinte; (c) `apply_voxel_cuts` atómico por corte e não por chamada; mais endurecimento de occupancy. Decisões: **push de c075d1f retido** até as correções entrarem (origin fica em d87fc65); correções sobre aeb7b39 com teste vermelho por achado; teste de invariante frame_at fresco == log == frame_at após tempo posterior, bit a bit, em todos os passos de um corte; todo teste de replay passa a afirmar que pelo menos um checkpoint foi tomado e usado, e as alegações de replay de P1/P2 são re-verificadas com essa afirmação. P4 parte pura (aeb7b39: remoção 6392 células = 99,875 m³ vs lei 100,928; 0,8/0,2; quantis) entregue junto com as correções.
- **Verificação de `c075d1f` limpa** (09:01–09:21): sr-sim 416, sr-eval 490, sr-model 122, sr-3d 114, varreduras 38, sr-gpu 522/6 GL, CLI 83/1 GL (as 7 falhas GL conhecidas do ambiente), sonda hero eeb71e14… (como em d87fc65), clippy, fmt, hygiene, sem marcadores de conflito. Push retido mesmo assim (defeito de ordem do P3 acima).
- **Pré-revisão de P4 (aeb7b39), 09:25**: confirmados remoção 6392 células = 99,875 m³ (kernel conservador 100,928, p≈2,86), teste da taça por ponto médio, 0,8/0,2 exato (5114/1278), velocidades por quantil monótonas, determinismo, colunas do rebordo de baixo para cima. Defeitos: o rebordo preenche por |r−crista| até ao tecto e sai como um anel de topo plano, não o perfil rim_height·bump (rim_height_at nunca usado); metade do rebordo da lei fica dentro da crista mas a remoção só usa bowl_depth_at, logo o lábio de voxels difere da superfície renderizada; velocidade mais alta à célula mais funda (Z-model lança o raso mais rápido); sem limite na caixa de varrimento do rebordo e sem verificação de unidades (kernel em unidades de objeto → ~5300³ células); ground_kernel usa p=2 (129,7 m³) sem manto. Testes: limite falso (19,1 m³), números não fixados, perfil do rebordo nunca verificado, flood fill aceita apoio lateral, tecto nunca exercido, quantis passam com baralhamento. Enviado a Netuno para corrigir com teste vermelho junto com as correções do P3.
- **Follow corrigido e integrado (09:32)**: `fix/follow-zero-window` = 80f70d5 (8c30823 chave do ruído; 80f70d5 política: uma face só pede espaço quando o ar na fumaça mais perto dela vai para ela), verificação local de Saturno limpa (sr-sim 418, sr-eval 490, clippy, fmt, corpus, hygiene). Previsão registada antes de medir e confirmada: a janela da hero NÃO fica em -78 até 3 s; medido com pyro_extent a 1/24 s: desce 2 células (−78 → −72) a t=1,125 s e fica até 4 s. Facto da cena: `boundary="open"` é do domínio inteiro (seis faces de pressão zero), a hero não tem colisores no pyro, logo não há face fechada: decisão (a), a janela mexe-se de verdade, a referência é medida em 80f70d5, a cena não muda para perseguir o hash antigo. Revisão minha dos dois diffs: chave interna = índice da caixa original (igual ao sem follow quando a janela está a zero), chaves externas com bit alto sem colisão com índices; a política soma o fluxo densidade×velocidade só nas lajes dentro da margem de cada extremo, sinal estrito (repouso não pede nada): sem achados. Merge --no-ff em main = **7dabb37**; verificação `vrun_7dabb37` lançada 09:32. Push continua retido pelas correções do P3.
- **Itens novos de Fase 3 (não agora)**: (i) uma face coberta por células sólidas (chão `colliders="solid"`) não perde massa e não deve pedir margem ao follow (teste: fumaça encostada a um chão sólido não move a janela); (ii) o domínio pyro da hero vai de y=−78 a 78 e a bola de fogo expande para baixo da superfície do mar sem obstáculo porque o pyro não vê o oceano: acoplamento oceano→pyro como fronteira sólida/água por célula.
- **Hero com follow, ANTES da correção (Mercurio, d87fc65, release, 720p)** em `/home/pals/renders/cinematic-impact/phase3/hero-follow/` com MANIFEST (nomes com o commit; não são referência): sonda t=3,0 = eeb71e14… contra 24c6ab56… (963b614, sem follow): 358 727/921 600 pixels (38,9 %), máx 159, média 0,896/canal, >8: 19 002, >32: 1366, massa da diferença na metade central (pluma e mar sob ela). Quadros 120/143 com vs sem follow: 364 546 (máx 131, média 0,833) e 334 242 (máx 68, média 0,939) pixels. **Massa nas 3 células do topo** (pyro_extent): t=5,0 0,30 / 0,30; **t=5,96 0,19 com follow / 88,54 sem follow** (de ~3000): sem follow a pluma é cortada pela face de cima; é o número que justifica o follow. A medição de referência repete-se em 80f70d5/7dabb37.
- **4.4 (Mercurio)**: revisão independente confirmou 9 achados, corrigidos em feat/foam-albedo: orçamento de memória cobrava espuma não desenhada (1b5b3f6); o raster REJEITA foamMode="albedo" com erro nomeado (ee3a6c1), XSD e SREP registam o limite (decisão: a mistura por cobertura no raster exige campos novos no uniforme e transmissão, não é mudança pequena). Faltam três de qualidade de teste e o fix/gl-skip (6 testes GL provados vermelhos; request_device em catch_unwind → GpuError::Device; testes saltam em Err).
- **Entrega de Netuno 7df5118 (09:40) integrada = main 28887eb**: sobre aeb7b39: 826cd43 (occupancy: chaves limitadas a [−2^30, 2^30), estimate_bytes em u128 saturante, brick repetido sempre erro, índice 0 da paleta), e9295a0 (`frame_at` aplica os cortes como `step_once`; teste do invariante fresco == pedido depois == replay de checkpoint sem log, bit a bit, 12 passos em torno de um corte a 1,5 s, falhava no passo 150; `World3::checkpoint_restores()` e todos os testes de replay afirmam restores ≥ passos para trás; P1/P2 não faziam alegação de replay, o P3 fazia e a entrada do ledger foi corrigida no lugar), f0653cb (parent_mass 0 fica fora via `voxel_spent`; aplicação em duas fases, tudo ou nada; registos impossíveis recusados; massa/inércia observadas por a=F/m e α=I⁻¹τ; a alegação de que o Rapier salta o recálculo de massa com o slot desativado não reproduziu), 7df5118 (P4: rebordo como conjunto de nível do perfil da lei, de baixo para cima com apoio por baixo; remoção sob a superfície crescida rim−bowl do kernel: 6460 células = 100,9375 m³ vs lei 100,928 (0,01 %), lançados 5168 / soerguidos 1292 / rebordo 1292; raso mais rápido a igual distância; i64 com verificação de intervalo; kernel em unidades de cena recusado com 'units'; a ligação deve construir o kernel conservador com bulking 1). Verificação local de Netuno limpa (sr-3d 119, sr-sim 424, sr-model 122, sr-eval 494, clippy, fmt, corpus, hygiene). Merge: c41bbbf (P3 que eu tinha integrado) não é antepassado da branch (reescrita como 0d82570); 7 trechos em conflito resolvidos pelo lado de Netuno; conferido que a árvore resultante = árvore de 7df5118 + exatamente os 5 ficheiros da correção do follow. Regra nova: nunca reescrever um commit já integrado. `vrun_7dabb37` abortada (superseded); `vrun_28887eb` lançada 09:44; revisão independente minha dos três commits em paralelo. Ledger 58 marcos.
- **Referência da hero adotada (09:50)**: sonda 720p t=3,0 em 80f70d5 (release, Mercurio) = **c8a664838a1d2e6845d6e392c33912dbd3c6859888bb3f3a655c942e3ae58c8b** (trace 10,33 s, parede 38,9 s, RSS 838 MiB); não voltou a 24c6ab56…, como previsto (a janela mexe-se a 1,125 s). Quadros sem follow idênticos aos de antes da correção (a9d92b13…, ea8a614c…: o caminho sem follow não mudou); com follow f120 = 1ad8be6d…, f143 = 90b9a0f3…. O script de verificação passa a comparar a sonda (perfil ci) com esta referência (PROBE_MATCH). Faltam de Mercurio: perfil ci, diferenças de pixels, topo por pyro_extent, MANIFEST.
- **Revisão independente minha de e9295a0/f0653cb/7df5118 (09:55)**: a maior parte confirmada. Dois DEFEITOS seguram o push: (1) a cauda de `frame_at` não regista o quadro e por isso pedir o mesmo instante duas vezes (ou um posterior depois dele) re-executa `apply_voxel_cuts` sobre um estado já alterado: com divisões encadeadas {0→[1,2]},{1→[3]} e o corte do filho no passo do corte do pai, a primeira chamada dá [T,T,F,T] e a segunda [T,T,F,F]; os testes não apanham porque o encadeado corta o filho a 1,0 e o invariante não tem cadeia; (2) a recusa 'units' é um limiar de tamanho (crista > 2000 células), não uma verificação de unidades: kernel em unidades de cena com crista de 4 m (1600 células) passa, dá cratera errada e `heap_rim` varre ~1e11 células. Qualidade de teste: inércia cega a rotações em x (peças com Iyy=Izz), teste sr-3d sem manto enquanto sr-eval usa manto (a alegação do lábio não está demonstrada para o kernel usado), comentários com números errados (3 % vs centésimo; 6459,39), replays de cadeia sem contar checkpoints além do primeiro. Enviado a Netuno; push retido até a correção.
- **Defeito de inércia encontrado por Netuno (09:58, d02582b sobre 7df5118)**: as propriedades de massa do Rapier/Parry para um corpo de voxels estão erradas para tensores diagonais com dois momentos iguais e o terceiro maior em x (bloco 3×4×4 células: Ixx 234,4 onde as células dão 300; placa 1×7×7 com furo 478 vs 937; a placa no mundo acelera a 0,0836 rad/s² onde o exato dá 0,0427). A alegação do P2 "Rapier = exato a 1e-12" era CIRCULAR (shape_mass_properties assentava na mesma chamada do Parry). Correção: `physics3d::voxel_mass` = momentos exatos i128 + Jacobi + referencial principal explícito dado ao Rapier para corpos de células, peças e parte que fica; testes vermelhos primeiro: mass_properties (17 proporções × 3 posições × 2 formas, 40 conjuntos aleatórios, cruz, placa com furo, caixa oca), world_inertia (spin-up vs I⁻¹τ, 5 formas × 4 torques + placa cortada de um bloco) e voxel_fracture (bloco a rodar partido em 3 fragmentos: momento linear e angular a 1e-12; era 256,3 vs 279,3). Ledger P2/P3 corrigidos no lugar + entrada do lado do mundo do P5. 28887eb (em verificação) tem a inércia antiga; d02582b entra com as correções dos meus achados numa só entrega.
- **Verificação de `28887eb` limpa** (09:44–10:11): sr-sim 426, sr-eval 494, sr-model 122, sr-3d 119, varreduras 38, sr-gpu 522/6 GL, CLI 83/1 GL, **sonda hero (perfil ci) = c8a66483…, igual ao release de Mercurio**, clippy, fmt, hygiene. Push retido pelos defeitos de inércia e de divisões encadeadas (entrega única de Netuno a caminho). **V.1 de Saturno entregue (10:05)**: feat/voxel-asset = fd5f74f, 7 commits sobre 28887eb (leitor .vox, cache SRVOL, voxelizador fromMesh, voxelAsset + voxels primitivos + VOX1–7 + corpus 373 docs, paleta por defeito + testes com os 13 ficheiros reais, secção da SREP com os quatro factos citados, correção de translações nTRN que transbordavam em i64 (teste vermelho) + testes contra os contratos novos do occupancy); verificação local limpa (sr-3d 141, sr-model 127, sr-eval 501, sr-sim 426, clippy, fmt, corpus, hygiene por commit). Sem o carregador do avaliador: decisão, merge de V.1 como está e o carregador + uso por render/física como V.1b (branch próprio, desenho de meia página antes, oráculos de massa/inércia e de cobertura raster vs path), antes do 3.2 pyroBlast. Revisão independente de fd5f74f a correr.
- **Entrega única de Netuno 3541fbe integrada = main 0c9c07e (10:13)**: d02582b (inércia exata de um corpo de células dada ao Rapier: `voxel_mass` com momentos i128 + Jacobi + referencial principal explícito; doc do módulo diz o que o Parry faz: `with_inertia_matrix` → `symmetric_eigen` lê diag(300, 234,375, 234,375) como diag(234,375, 300, 234,375); exemplo `parry_inertia_defect`; ledger P2/P3 dizem que a verificação antiga era circular), 6fdf02c (`slot_since`: um slot é cortado a partir do passo seguinte ao uso, seja qual for a história de pedidos; teste `the_same_four_ways` nos casos simples e encadeado, com checkpoint afirmado), dc6ffff (`excavate(before, kernel, kernel_unit, h, ejection)`: unidade declarada, tecto de 2^27 células que erra nomeando a caixa; kernel em meias-unidades dá o mesmo ao bit, em unidades de cena concorda a 8 células, crista de 1600 células recusada em < 1 s), 3541fbe (peça em L com produtos de inércia; `Crater::mantle_height_at` e mapa = bowl + rim + manto a 1e-12; o ledger diz que a cratera de voxels segue bowl e rim e NÃO o manto; 6459,36 calculado, 6460 fixado; replays com checkpoints além do primeiro). Verificação local limpa (sr-3d 120, sr-sim 426, sr-model 122, sr-eval 499). Ledger 59 marcos. `vrun_0c9c07e` lançada 10:13; revisão independente dos 4 commits a correr.
- **Hero-follow, conjunto de referência final (Mercurio, 80f70d5, 10:14)** em `/home/pals/renders/cinematic-impact/phase3/hero-follow/` (MANIFEST.txt): sonda t=3,0 release = ci = c8a66483…; pixels: corrigido vs antes da correção 185 284 (20,1 %), máx 157, média 0,153; vs referência antiga sem follow 345 832 (37,5 %), máx 159; quadro 120 com vs sem follow 355 186 (38,5 %), máx 102; quadro 143 330 453 (35,9 %), máx 69; corrigido vs antes nos quadros ~197 000, média 0,143; quadros sem follow bit-idênticos entre d87fc65 e 80f70d5. Topo (3 células): t=5,0 0,27 / 0,30; t=5,96 **0,17 com follow / 88,54 sem** (antes da correção 0,19). Conclusão: sem follow a pluma é cortada pelo topo; a correção não piorou nada. Aceite como evidência (não é entrega para assinatura).
- **fix/gl-skip, diagnóstico revisto (Mercurio)**: nesta máquina o device GL abre; o pânico vem da primeira pipeline com buffers de armazenamento (GL anterior a 4.3: BUFFER_STORAGE | DYNAMIC_ARRAY_SIZE); correção = sonda mínima em `Gpu::open` (pipeline com buffer de armazenamento e arrayLength em escopos de erro) que devolve GpuError::Device; os 6 testes saltam. Condições minhas: custo do open na NVIDIA ≤ 5 ms medido; varredura completa sr-gpu/CLI na NVIDIA com os mesmos 522/83 e os 7 GL a saltar.
- **V.1b (carregador de voxelAsset, Saturno) — desenho aprovado (10:16)** com três ajustes: `VoxelModel.origin_cells` guarda a chave mínima no referencial do ficheiro (a origem do objeto é o canto mínimo, dito no XSD; o pivô floor(size/2) reconstrói-se); a cache por (programa, chave) só reutiliza se os bytes/sha256 baterem; o oráculo de cobertura raster vs caminho exige ≤ 1 % da área E só pixels de borda. Ordem: carregador + oráculos de sha256/limites/massa 2×2×2 → `sr_3d::voxel::faces` + oráculo 2(ab+bc+ca) → malha com Mercurio + cobertura. Branch feat/voxel-assets-eval sobre main após o merge de V.1.
- **V.3 (superfície de voxels, Mercurio) — nota de desenho em `/home/pals/renders/cinematic-impact/phase3/voxels/V3-surface-design-note.md` (10:18) e decisões**: módulo puro `sr_3d::voxel` (faces expostas por bitboard de brick, fusão gulosa determinística por plano; cubo n³ = 6 quads, furo passante = 16; dourados FNV-1a; recalcula só quando (lineage, revision, partição de classes) muda; orçamento surfaceMemoryMiB 128; cor de vértice, nenhum shader muda, identidade das cenas sem voxels por construção). Decisões: (1) revision + lineage() aprovado, Mercurio escreve, Netuno revê; (2) emissão E1 aprovado; (3) vidro–vidro = uma face, dono a menor classe, documentado; (4) classes de equivalência na V1; (5) peças por faixa canónica; (6) 128 MiB; (8) cellSize pequeno = aviso Schematron; **(7) recusado**: a cratera em chão de células (P4) existe, um corte é um evento de revision e o caminho incremental tem de cobri-lo (oráculo: corte de k bricks remalha só k e iguala a remalha completa). Tempos só após sonda `voxel_mesh_seconds`. Propriedade: faces() é de Mercurio (Saturno não a escreve; o oráculo 2(ab+bc+ca) vira teste do módulo dele). Ordem de Mercurio: fix/gl-skip e feat/foam-albedo primeiro, depois feat/voxel-surface (11 commits TDD, sonda de pixels das junções em T antes de fechar).
- **Revisão independente de V.1 (fd5f74f), 10:25**: confirmado por decodificação python dos ficheiros reais (axes 12 452 células, extremos, checksum, cubo de 332; MATL; versões; byte 105; composição; paleta; c→c−1; overflow; 270 asserções; 14 docs VOX); limpos: chunk walk, truncagem, XYZI, SRVOL, contratos do occupancy, voxelizador. **Defeitos**: passeio do grafo exponencial antes do cap (filhos partilhados: 2^30 colocações com ~2 KB); rotação com sinal negativo uma célula ao lado (reader p=s·(x−c) vs convenção de caixa do ogt_vox c−x−1; make_vox partilha a convenção, fixtures concordam por construção; nenhum ficheiro real tem rotação); Schematron VOX2 com substring −4 em vez de −5 (um .srvol válido falha; sem doc no corpus); grafo só de ciclos importado sem erro, nós fora da raiz não validados; memória do voxelizador (rows/filled) não limitada por max_bytes (placa 16384² → ~8 GiB); unidades do fromMesh por declarar (malhas a 100 u/m, y invertido; regra: voxelizar no referencial importado); SRVOL sem sha256 da fonte (fica para V.1b). Qualidade de teste: paleta comparada consigo própria, contagens reais a 0 (563/294/398), overflow sem posição, dois testes que não podem falhar; nits de contagem (377) e README. Enviado a Saturno; merge de V.1 só depois.
- **Revisão independente de d02582b..3541fbe (10:30)**: confirmados os números do bloco 3×4×4 (1800 kg, diag(300, 234,375, 234,375)), Jacobi, i128, uso no mundo/peças/parte que fica, `slot_since` em State, `the_same_four_ways`, unidade e tecto, peça em L sensível a troca y↔z e ao sinal de Ixy. Causa raiz confirmada e mais precisa: glamx-0.3.1 eigen3.rs:144 devolve X sempre que o menor valor próprio é repetido. **Defeitos**: fracture.rs:100 ainda soma as propriedades das fontes de Fracture3 pelo Sum do Parry (with_inertia_matrix): fonte 3×4×4 de dois fragmentos 3×4×2 simula com Ixx 234,375 até fraturar (voxel_fracture não apanha porque a sua fonte 8×4×4 tem o momento distinto menor); `keys_of` depende do intervalo semiaberto de voxels() do Parry e salta a chave 687 com células de 0,1 m (688·0,1/0,1 = 687,999…). Qualidade de teste: mass_properties compara voxel_tensor com uma cópia de Moments::properties e não exercita principal()/frame; Jacobi só roda a rotação (0,1) (peça em L); verificação de momento angular do voxel_fracture é uma identidade sob as próprias asserções; caso largo do crater só is_err(); caso 0,01 só compara contagens. Enviado a Netuno como tip pequeno; **push continua retido** (a frase "every path" do ledger é falsa até lá).
- **Verificação de `0c9c07e` limpa** (10:13–10:29): sr-sim 428, sr-eval 499, sr-model 122, sr-3d 120, varreduras 38, sr-gpu 522/6 GL, CLI 83/1 GL, sonda = c8a66483… (PROBE_MATCH=yes), clippy, fmt, hygiene. Push retido só pelo tip pequeno de Netuno (fonte de Fracture3 e keys_of).
- **fix/gl-skip integrado = main f6750db (10:35)**: 00e1d1a (1 commit; gpu.rs + cli.rs): sonda mínima em `Gpu::open` (fragmento com buffer de armazenamento de tamanho dinâmico + arrayLength em escopos Internal e Validation) devolve GpuError::Device com o motivo; os 6 testes GL e o da CLI saltam. Medido na NVIDIA: abrir device mediana 130 ms com a sonda vs 131–138 sem (não mensurável); varredura completa sr-gpu 528/0, CLI 68/0, scene-render 16/0; `backends` termina em 1 s (antes horas). Revisto por mim: nit (o escopo interno não é despejado quando a validação já falhou; seguimento pequeno). **feat/foam-albedo = 9a5867e** (8 commits sobre 28887eb) entregue: identidade dos sete quadros e da sonda hero (c8a66483…), f=0 sem custo (10,29 vs 10,19 s), raster recusa albedo com erro nomeado, orçamento sem espuma não desenhada, SREP com a linha do custo (21,3 s vs 3,2 s a 640×360 se a variante da água fosse forçada); revisão independente a correr; integra com o tip de Netuno numa só verificação. Mercurio começa V.3 em feat/voxel-surface sobre 0c9c07e.
- **Revisão independente de feat/foam-albedo 9a5867e (10:40)**: confirmados mistura convexa, gancho antes de Fresnel, f=0 exato, slots, cobertura determinística, orçamento sem discos, rejeição do raster, XSD/corpus, teste de rugosidade. **Defeitos** (merge espera): guia de albedo do denoiser escrito antes do gancho (borda espuma/água sem proteção; todos os testes com denoise=false); luz refratada leva a cor da espuma (thr *= albedo misturado em dielectric_crossing e first_interface: água a 0,5 transmite ~0,45 em vez de ~0,01–0,05); custo da cobertura (9 bins por traçador, foamRadius sem limite) e memória (HashMap ~80 B/vértice) não orçamentados; no raster com água misturada o erro é empilhado mas foam_mix fica None e a água é desenhada com alfa = parte. Qualidade de teste: montagem do pipeline sem teste (a regressão do custo de 21,3 s passaria), teste do documento só direção e só foamAlbedo, f=0 só na água opaca. Docs: linha do custo irreprodutível; faltam avisos (foamMaterial em albedo; albedo sem câmara pathtrace), câmara no doc válido, tabela de atributos. Nit: a identidade do shader da água é medida (NVIDIA), não estrutural (`override FOAM` entra em todas as cenas de água). Enviado a Mercurio; foam antes de V.3.
- **Push a origin por ordem do usuário (14:31, após ~3,5 h de pausa)**: `d87fc65..f6750db` (22 commits: correção do follow 8c30823/80f70d5, P3 corrigido, P4, inércia exata, tecto de unidades, gl-skip). 0c9c07e verificado limpo; f6750db = 0c9c07e + 00e1d1a (77 linhas, revisto por mim, varredura NVIDIA de Mercurio 528/0 e 68/0); `vrun_f6750db` lançada depois do push para o registo. Em aberto para o próximo push: tip de Netuno (fonte de Fracture3, keys_of), V.1 corrigido (Saturno), foam corrigido (Mercurio).
- **V.1 corrigido integrado = main 464d3a0 (14:33)**: feat/voxel-asset = 7537c40, 5 commits sobre fd5f74f: 8710dfb (contagem memoizada e saturante de colocações por nó antes de colocar, `Bounds::max_placements`; 30 níveis com filhos partilhados erram em < 5 s nomeando 1073741824; exatamente uma raiz que alcança todos os nós, sem raiz/duas raízes/nó fora/nó ou modelo em falta são erros nomeados), b3a2aba (eixo negado manda q para −q−1, convenção da caixa unitária do ogt_vox.h 123–170; valores à mão: byte 105, 3³, (0,0,0), t=(5,6,7) → (4,6,7), reader antigo dava (4,7,8); make_vox.py reescrito com centros fracionários independentes do reader; SREP: nenhum ficheiro real tem rotação), 4fbde80 (VOX2 −5; 3 docs novos; corpus 380), 61e854b (varrimento do voxelizador por blocos `Bounds::chunk_cells`, resultado igual para blocos de 1, 7, 20, 1000, 2^20; fromMesh via `voxel::from_model` no referencial do render (basis × nó por model_triangles, o mesmo dos colisores): cubo glb de 1 m cortado a 10 → 1000 células x 0..9, y −10..−1, z −10..−1; regra no XSD e SREP), 7537c40 (paleta fixada a literais da spec em 8 índices; contagens 563/294/398; índice 0 com ficheiro de 256 entradas; round trip SRVOL dos 255 índices; overflow verifica a célula (6,0,−1); MAX_SIDE 256; tools/vox_dump.py e vox_axes.py reproduzem 12452 células, checksum, cubo 332; SRVOL sem sha256 escrito como limitação). Verificação local de Saturno no rebase equivalente (5b15b01): sr-3d 147, sr-model 127, sr-eval 508, sr-sim 428, clippy, fmt, corpus 380, hygiene. `vrun_464d3a0` em fila atrás de `vrun_f6750db`; revisão independente dos 5 commits a correr.
- **Revisão dos 5 commits de correção de V.1 (14:40)**: quatro confirmados ao detalhe (bytes de rotação à mão, make_vox regenerado byte-idêntico, VOX2/corpus 380, referencial diag(100,−100,−100), paleta = fórmula da spec, contagens, vox_axes). **Um defeito**: `walk` ainda constrói um Vec de todas as colocações; com max_placements 2^24 e modelos vazios aceites, ~1,5 KB fazem 2^24 colocações e ~640 MiB–1,3 GB; com Limits::default() (max_cells u64::MAX) 255 voxels × 2^24 enchem o BTreeMap. Correção pedida (fix/vox-walk sobre 464d3a0, antes do próximo push): colocar enquanto percorre, total de células Σ colocações×células verificado antes contra max_cells e contra um limite absoluto do importador, max_placements 2^20. Qualidade: mensagem sem raiz só contains("root"); blocos 1/7/20 equivalentes (linha de 21 células); RGBA uniforme não distingue deslocamento; exemplo da SREP com cellSize 0,25 (64 M células).
- **Entrega 2 de Netuno b89f0d3 integrada = main 27cb496 (14:41)**: 8 commits sobre 3541fbe: 9b154d5 (cortes aplicados antes das cargas lerem os corpos em step_once, e de novo depois; uma carga via o corpo pré-corte, achado do workflow próprio de 61 agentes), 8aca290 (`sum_mass_properties` exato só quando todos os fragmentos são corpos de células; exclusividade divisão/fratura nos dois sentidos; doc com a causa raiz do glamx), 73d6720 (`State.voxel_cells`: células ordenadas de cada corpo cortável, partilhadas com os checkpoints; cortes validados; keys_of apagado; vermelho 1,32 vs 0,86 rad/s²), 907cfd9 (peça com colisor DisabledByParent não tinha massa no primeiro passo: COM 0 vs 2,25), 3bdad48 (campos lidos e aplicados no centro de massa para corpos de células; 0 vs 0,4 m/s), 0b3375e/c3e8335 (frame principal lido de volta; corpos irregulares com produtos ≥ 1e-3 do traço; mutações falham 4–5 testes; `last_restored_checkpoint`; conjuntos de células nos testes da cratera; corte em função do impacto; escala ppm 100 com células 25×50×12,5), b89f0d3 (ângulo/dispersão NaN recusados; pai sem células recusado; alcance por hipotenusa; helper sem overflow). **Correção honesta**: em chão plano a remoção é 6352 células = 99,25 m³ = 98,34 % da lei (6460 só com o pilar de 15 m); ambos fixados e no ledger. Verificação local limpa (sr-3d 120, sr-sim 432, sr-model 122, sr-eval 505). Decisões: corrigir também o Sum do Parry nas fraturas de malha (commit próprio, vermelho primeiro; listar cenas com fraturas de malha; Mercurio re-mede a sonda hero e os sete quadros; se os bits mudarem, novo hash de referência com a causa registada) ANTES de `VoxelCut3.added`; `added` aprovado vermelho primeiro (células vazias antes e dentro dos limites; massa e tensor = momentos exatos do conjunto novo; checkpoints; quatro caminhos iguais). `vrun_27cb496` em cadeia atrás de `vrun_f6750db`; revisão independente dos 8 commits a correr.
- **Revisão independente de 3541fbe..b89f0d3 (14:50)**: confirmados (termo de eixos paralelos, Jacobi/colunas, ramo só-Voxels na soma da fratura, exclusividade, voxel_cells ordenado com erros, switch do colisor só em slots, campo só em Shape3::Voxels, leitura de volta do tensor, 6352/6460). **Defeitos** (próximo tip, com o commit do Sum das malhas): (1) novo em 9b154d5: step_once corta antes de sync_visibility e a cauda de frame_at faz o contrário: corpo nascido e cortado no mesmo passo dá peças com pose pré-nascimento num caminho e com pose de nascimento no outro (a mesma assimetria antiga para cargas que leem corpos nascidos no passo); correção: a sequência completa da cauda uma vez antes das cargas; caso de nascimento em the_same_four_ways. (2) residual de 907cfd9: peça escondida ao cortar fica sem massa no primeiro passo em que aparece (recompute só com shows=true; sync_visibility não recomputa até pipeline.step). (3) checkpoint_charge não conta voxel_cells (12 B/célula, novo Arc por corte, cópia no registo). Qualidade: teste do eixo inclinado já era verde antes do hypot; asserção disjuntiva no Undermine; `< 0.02` redundante. Nits: idempotência do pai assenta em revision == installed (contrato do driver a declarar); apply_voxel_cuts duas vezes por passo clona slots.
- **Erro de processo meu (14:50)**: fiz merges (464d3a0, 27cb496) no worktree enquanto `vrun_f6750db` corria nele; o script só confere o HEAD no início, logo sr-eval/sr-model/sr-3d dessa corrida testaram árvores posteriores. Abortada e marcada; f6750db fica apoiado em 0c9c07e limpo + 00e1d1a revisto e varrido por Mercurio na NVIDIA. `vrun_27cb496` arrancou às 14:50 com o HEAD certo. Regra: nunca mover o HEAD do worktree durante uma verificação (memória).
- **fix/vox-walk = 841447f (Saturno, 14:55), sobre 464d3a0, revisto por mim e aprovado**: passeio por callback sem Vec; grelha construída no lugar sob min(limite do chamador, limite do importador: 2^26 células = tecto do maxCells do XSD, 2^30 bytes); max_placements 2^20; teste vermelho (24 níveis com modelo vazio: antes Ok em 1,2 s com o Vec, agora recusado em < 1 s nomeando 16777216 e o limite 1048576); 2^20 × 255 voxels recusado nomeando 267386880 e 67108864; mensagens exatas nos 5 casos do grafo; blocos 1/7/20/21/126/987/9261/2^20 com os regimes ditos; RGBA distinto por entrada; round trip SRVOL por fingerprint; exemplo da SREP com cellSize 5. Verificação local limpa (sr-3d 148, sr-model 127, sr-eval 508, sr-sim 428). Merge em main adiado até `vrun_27cb496` terminar. Saturno confirmou com pyro_extent os números do follow (janela −78/−72/−75/−90 a 1/1,25/5/5,96 s; topo 0,2745 e 0,1706) e escreve o marco do ledger em docs/follow-ledger.
- **docs/follow-ledger = bf2bb02 (Saturno, 15:00)**: marco "pyro@follow…" no ledger e a SREP com os três hashes da sonda e as razões (24c6ab56 sem follow; eeb71e14 follow fundido; c8a66483 após 8c30823 e 80f70d5), a janela medida (−78 → −72 a 1,125 s; −75 a 5 s; −90 a 5,96 s), topo 0,17 vs 88,54, pixels de Mercurio. Revisão minha: uma frase sem apoio ("a primeira política moveu a janela da hero 22 unidades no sentido errado"): as medições de Saturno dão a MESMA trajetória da janela na hero nas duas políticas; a política nova justifica-se pelo oráculo da fumaça em repouso; os 185 284 px entre eeb71e14 e c8a66483 atribuem-se à chave do ruído salvo medição. Correção pedida em commit novo; integra com fix/vox-walk depois de `vrun_27cb496`.
- **docs/follow-ledger corrigido = 9102045 (15:05)**: frase dos 22 unidades retirada da SREP e do marco; ambos dizem que a janela da hero é a mesma sob d87fc65 e 80f70d5 (6 unidades abaixo a ~1,1 s nas duas), que a política das faces se justifica pelo oráculo da fumaça em repouso e que os 185 284 px se atribuem à chave do ruído salvo medição (sonda só com 8c30823, nomeada e não feita). Aceite; integra com fix/vox-walk após `vrun_27cb496`. Saturno começou V.1b em feat/voxel-assets-eval sobre 841447f.
- **V.1b (carregador) entregue por Saturno (15:10)**: feat/voxel-assets-eval = 153ee5f, 2 commits sobre 841447f: a220690 (materiais .vox como números + fingerprint, sr-3d), 153ee5f (`voxel_asset::load(program, key) → Arc<VoxelModel>` com occupancy no canto mínimo, origin_cells, colours, materials, fingerprints, cell_size, source{sha256, bytes, modified}; sha256 verificado antes de qualquer parse; limites 4 194 304 / 128 MiB nomeados; cache por programa e chave reutilizada só com o mesmo sha256 dos bytes; registo no Program via object3D primitive="voxels"). Oráculos: cubo 2×2×2 de .vox → body → massa 8ρs³ e inércia mL²/6 a 1e-12; modelo sob dois grafos; sha/limites/remoto/extensão; cache; materiais e fingerprint (5+9 edições); glb de 1 m → 1000 células, origin_cells [0,−10,−10]; SRVOL a 0,5. Verificação local limpa (sr-3d 153, sr-model 127, sr-eval 515, sr-sim 428). Desvio auto-declarado: o script de Saturno lançou binários de sr-gpu fora da fila por ~3 min, cortados por PID; não conta como verificação. Falta (de terceiros): cobertura raster vs caminho (superfície de Mercurio) e rigidBody@shape="voxels" (Netuno). Revisão independente a correr; Saturno segue com a nota do PieceGraph (Netuno) e depois 3.2 pyroBlast com desenho antes.
- **Verificação de `27cb496` limpa** (14:50–15:18): sr-sim 434, sr-eval 514, sr-model 127, sr-3d 147, varreduras 38, **sr-gpu 528/0 e CLI 84/0 (os 7 testes GL agora saltam)**, PROBE_MATCH, clippy, fmt, hygiene. Em seguida merges de fix/vox-walk (841447f) e docs/follow-ledger (9102045) = **main ba87333** (ledger 60 marcos); `vrun_ba87333` lançada 15:19. Push depois dela e do tip de Netuno (o ledger de 9b154d5 diz "finds the same body however the step is reached", falso até à correção da ordem de nascimento).
- **Revisão independente de V.1b (153ee5f), 15:25**: confirmados campos, origem, sha antes do parse, limites, cache por hash, preguiça, fingerprint por to_bits. **Defeitos** (latentes: nada chama load ainda; corrigir antes do merge): fromMesh em include procura a chave crua em vez de `ns/r` (falha ou corta a malha errada); fromMesh sem verificação de digest e ficheiro lido duas vezes (Source.sha256 pode não ser dos bytes cortados; .bin externo de .gltf fora do digest); modelos com largura ≥ 2^30 falham em Occupancy::set sem erro nomeado. Qualidade: contains("remote") passa com qualquer erro (o id é `remote`); teste dos dois grafos sem contagem de células; fingerprint dos materiais circular. Nits: cache re-hasheia o ficheiro a cada acerto; pico ~3× maxMemoryMiB; `_ri` vs `_ior` só no doc; −0/0. Enviado a Saturno.
- **Verificação de `ba87333` limpa** (15:19–15:31): sr-sim 434, sr-eval 514, sr-model 127, sr-3d 148, varreduras 38, sr-gpu 528/0, CLI 84/0, PROBE_MATCH, clippy, fmt, hygiene. Push aguarda o tip de Netuno (ordem de nascimento, peça escondida, carga dos checkpoints, Sum das malhas) para não levar a frase falsa do ledger; Netuno e Mercurio já têm commits novos nos branches (0025b94, 7e10653) sem mensagem de entrega ainda.
- **Entrega 3 de Netuno 1fd0fe5 integrada = main 966123a (15:37)**: 8 commits sobre b89f0d3: 4aeae6b (soma exata dos tensores das peças para a fonte de TODA fratura), 451f8f8 (tensores das peças de malha exatos, não from_trimesh), 27be102 (o passo começa com a sequência da cauda: corpo nascido e cortado no mesmo passo igual em qualquer caminho; caso em the_same_four_ways), 590c78d (sync_visibility liga/desliga o colisor e recalcula a massa: peça escondida ao cortar e fragmento escondido ao nascer com massa e COM no primeiro passo visível; vermelho COM 0 vs 2,25), 0025b94 (checkpoint cobra voxel_cells a 12 B/célula; 14 MB por checkpoint num piso de 1e6 células; ledger corrigido), 2f3b7be (cratera larga inclinada a 45° com 480 000 células vermelha no alcance antigo, 64 440 células em falta; Undermine com v_y ≈ g·3·dt; doc do trait voxel_cut; teste do driver com revisão nova passou à primeira, declarado), 95cb04b (ledger: fraturas de malha mudam nos bits baixos; nenhum hash dourado mexeu; impact-block é o único exemplo com fratura), 1fd0fe5 (fmt). `vrun_966123a` lançada 15:37; revisão independente a correr; push depois dos dois. Netuno segue para VoxelCut3.added e P5 sobre sr_3d::pieces (109ff17 de Saturno).
- **Entrega 4 de Netuno be86475 (15:45), em fila para merge após `vrun_966123a`**: bb83ccd (`VoxelCut3::added`: células amontoadas no corpo antes de qualquer saída; já existente ou repetida = erro e nada instalado; vermelhos: COM com as células, COM com metade do monte destruída no mesmo corte, recusas, bola em repouso a 0,8 m sobre o monte; o teste de replay passou sem a implementação, declarado), be86475 (`voxel_crater::crater_cut(before, kernel, kernel_unit, ejection, revision, Rock, anchored) → CraterCut{excavation, cut}`: cratera no slab com pilar = 1292 adicionadas, 6460 destruídas, topo do pilar (132 células) é peça; cada célula e cada kg num só sítio; mundo de 576 000 células com COM a 1e-6 m; erros para células não cúbicas, unidade 0, terreno vazio; ledger P4 diz o que falta: chão inclinado e eixo fora do plano x-y; nenhuma cena atira a cratera a um corpo até CRT13/esquema de Saturno). Verificação local parcial (sr-eval voxels 25/25, clippy sr-sim+sr-eval, fmt, hygiene). **feat/pieces = 109ff17 (Saturno, sr_3d::pieces, 474 linhas sobre ba87333)** sem mensagem de entrega: revisão independente lançada; Netuno baseia o P5 em 109ff17 diretamente; pedi confirmação do tip a Saturno.
- **Revisão independente de b89f0d3..1fd0fe5 (15:55): sem bloqueio**; confirmados tetraedros/valores à mão/soma em toda fratura/ordem prefixo = cauda/só transição escondido→visível/aritmética da carga/hypot/Undermine. Para o próximo tip: mesh_mass_properties sem verificação de fecho/orientação (peça aberta dá tensor dependente da origem); fragmentos Shape3::Convex ainda pelo Parry (eixos trocados num convexo em caixa); teste do driver Counting corta desde t=0 e nunca chega à cauda; comentário :1459 e doc :217 desatualizados; nota [MESH FRACTURE] no sítio errado do ledger e "nenhum hash dourado mexeu" vazio (nenhum cobre impact-block); mudança de comportamento (cargas leem fragmentos no passo da fratura) a registar; limiar de ~1,9e7 células acima do qual nenhum checkpoint cabe em 256 MB; len vs capacity; metade "fragmento escondido" de 590c78d provavelmente não era defeito (só slots).
- **Revisão independente de sr_3d::pieces 109ff17 (16:00)**: confirmados invariantes (célula numa só peça, face contada uma vez, a < b, face_sum e normal rederivados, ordem total, custo). **Defeitos**: área da junta errada para células não cúbicas (faces por eixo); overflow i64 no Voronoi com sementes grandes; overflow no offset do plano; `seeds_in` divide por zero; sementes sem tecto (~96 GB). API: Piece sem Moments (Netuno recalcula), arestas sem centros, piece_of deitado fora, campos pub, HashMap. Testes: um no-op, blob sem verificar a semente certa, ordem fraca, check com o próprio components. SREP sem propriedade partilhada nem limites. Enviado a Saturno (commits sobre 109ff17) e a Netuno (P5 sobre 109ff17, merge dos commits depois).
- **Disco cheio durante `vrun_966123a` (≈15:50)**: Saturno relatou o disco cheio por minutos; a corrida mostrou sr-3d 129/19 só em testes de importação de assets (gltf/fbx/usd/materialx com ficheiros temporários), coerente com ENOSPC, não com os commits (sr-sim/sr-eval). Ação: apagado target/release do meu worktree (18 GB; só uso ci), 61 → ~79 GB livres; regra para os agentes: `df -h` antes de verificações e apagar debug/release do próprio worktree abaixo de 40 GB. sr-3d será repetido isolado após a corrida para confirmar. **Tips de Saturno recebidos**: feat/pieces 9d5d476 (faces por eixo + área/centroide por tamanho; Voronoi em i128 com sementes ≤ 2^40 e ≤ 4096; planos sem subtração; seeds_in com Result; Piece::moments(); piece_of; campos privados; sem HashMap; blob contra argmin por força bruta; testes de extremos; SREP com dois autores) e feat/voxel-assets-eval 018261b (fromMesh em include pelo namespace do voxelAsset; sha256 do mesh verificado e digest sobre mesh + ficheiros lidos (.bin, mtl/texturas), recalculado após o corte; at_origin com erro nomeado; fingerprint dos materiais fixado por tools/vox_materials_hash.py; pré-filtro tamanho+mtime na cache; −0=0). Revisão independente dos dois a correr; merges em fila.
- **3.2 pyroBlast — desenho de Saturno (16:10) aprovado com três condições**: `<pyroBlast time x y z energy>` filho de `<pyro>`; frente de Sedov–Taylor R(t)=ξ(γ)(E t²/ρ0)^{1/5} com ξ por quadratura (oráculo: 1,033 a γ=1,4; 1,15 a 5/3; energia = E a 1e-6); solver incompressível carrega só o deslocamento do ar como êmbolo esférico (divergência (V_{n+1}−V_n)/(V_{n+1} dt) na esfera; escoamento potencial fora); R_max = 0,3 (E/p0)^{1/3}; sem calor nem fumo; cinco limites; oráculos de conservação do ar, energia cinética do êmbolo (duas resoluções), sobrepressão RH de ordem de grandeza, identidade sem blast (9 hashes do solver). Condições: (1) resolução temporal: com dt=1/24 s e E ~ 1e15 J, R(dt) ≈ 280 m > domínio: sub-passos internos do êmbolo ou declaração do limiar de energia acima do qual é um pulso de um passo, com tabela R(dt), R(2dt), R_max para três energias; (2) domínio fechado + fonte líquida de divergência = erro do avaliador (regra PYC) ou compensação dita; (3) unidades explícitas (cena vs metros via pixelsPerMeter; teste com ppm ≠ 1). Acoplamentos a corpos (A·p_s·τ) e oceano (waterImpulse com ∫p) em commits posteriores. Branch feat/pyro-blast sobre 966123a; sedov primeiro, red-first.
- **Revisões (16:02)**: feat/pieces 9d5d476 sem defeitos (anel confirmado à mão, sem overflow, determinismo); nits (extremos só com check(), z do centroide, Edge::new sem validação, f64 acima de 2^53) → merge na próxima janela. V.1b 018261b: 4 defeitos pequenos antes do merge: dependencies_as lê e parseia a malha inteira antes da cache e sem limite; stat depois da leitura (corrida); dependências em falta fora de stats (criar uma não invalida); pico de memória subestimado para fromMesh; CLI do vox_materials_hash.py falha sem materiais. Confirmados namespace, fingerprint 10385904548290088009 reproduzido pelo python, −0, ordem sha→corte→digest.
- **pyroBlast: condições respondidas por Saturno (16:03) e aceites**: módulo sedov em feat/pyro-blast ac2b4b2 (ξ0(1,4)=1,03278, ξ0(5/3)=1,15167 por integração das EDO de semelhança; python independente a 2e-6; 6/6). (1) Tempo: R(dt), R(2dt), R_max para E = 1e6/1e9/1e15 J (4,43/5,84/0,64 m; 17,6/23,3/6,44 m; 279/369/643 m); a fase forte acaba dentro do primeiro passo para E < E* = [ξ0 (dt²/ρ0)^{1/5} p0^{1/3}/0,3]^{15/2} ≈ 1,4e12 J a dt=1/24; decisão: sem sub-passos (o campo potencial só vê o volume varrido), R_{n+1} = min(R_Sedov, R_max), pulso de um passo abaixo de E*; divergência ΔV/(V dt) em todas as células cobertas, domínio recortado dito e testado; a hero não usa blast. (2) Domínio fechado: erro em três níveis (PYC, Rust, avaliador). (3) Unidades: cena para x y z, SI para E/ρ0/p0, metros por unidade = 1/pixelsPerMeter; teste a ppm 1/100/37. Pedido meu: oráculo 6 (casca de fumaça a dois raios desloca-se ΔV/(4πr²) e conserva massa a 1e-6 num pulso com CFL ≫ 1; a SREP diz o que a advecção faz nesse passo). Ordem de Saturno: defeitos de V.1b → nits de pieces → solver do blast.
- **Entrega 5 de Netuno f00dddd + P5 aa46dfc (16:05)**: d59d0c2 (malha aberta ou enrolada nos dois sentidos recusada; cantos soltos soldados por posição aceites; fragmentos Convex pelo hull → triângulos → integrais exatos: vermelho 5,647 rad/s² onde a placa dá 3), 66efbd6 (Counting corta a partir de 0,03 s; comentários; carga dos checkpoints por capacity com shrink_to_fit, vermelho 16,336 B/célula), f00dddd (nota da fratura no sítio certo; "nenhum hash dourado cobre fratura"); registados: cargas leem fragmentos no passo da fratura, falha de load deixa a fratura aplicada, limiar ~1,9e7 células a 256 MiB, metade "fragmento escondido" nunca foi defeito. **P5** feat/voxel-fracture (3da33bf, aa46dfc; contém 966123a + f00dddd + 9d5d476): `voxels::fracture(occupancy, Partition, max_pieces, size, density, ppm) → Fractured{source, pieces, graph}`; oráculos escritos antes: cada célula e cada kg numa só peça (1e-9), ordem da partição, igual para qualquer ordem de entrada, erros, peças como fragmentos de Fracture3 no lugar da fonte. Não feito: política de overflow/fragmentMinCells; nenhuma cena liga (esquema CRT13 de Saturno). A máquina esteve a 0 MB livres um momento (Netuno limpou o incremental). Revisão independente dos dois a correr; ordem de merge: f00dddd + 9d5d476 → verificação → push; P5 depois da revisão.
- **Mercurio, ponto de situação (16:06)**: feat/foam-albedo com 13 commits sobre 966123a (rebaseado; nada estava em main): os 4 defeitos e os 3 de qualidade tratados com teste vermelho; **mudança de modelo aprovada**: a parte é área coberta e cada amostra está na espuma com essa probabilidade (espuma difusa branca sem transmissão nem metal; água intacta com o seu tinto; sombra refratada com 1 − parte; guia do denoiser = albedo médio; f=0 bit a bit também na água transmissiva); medido que a mistura antiga não era linear (0,0283/0,0552/0,1042 vs médias 0,0526/0,0996/0,1462); com o novo: linear a 5 %, água vista de baixo a 0,3 % (antes 6 %), borda com denoiser 0,0025 (antes 0,0071); cobertura orçamentada (maxWork, 80 B/vértice em surfaceMemoryMiB, OCN14 foamRadius ≤ 64 células); raster recusado não desenha água; W06/W07; SREP com custo reprodutível e identidade "medida". Verificação CPU limpa (805); GPU na fila. Pedidos meus: ruído a f=0,5 com e sem denoiser vs mistura antiga na SREP; a aproximação da sombra dita. **V.3** feat/voxel-surface: 5 commits TDD (lineage(), faces expostas com oráculos 6n² etc. e 2(ab+bc+ca), fusão gulosa com dourados do mesher de referência em python, expansão para Vertex de 96 B, orçamento 1240 B/quad); módulo em `sr_3d::voxel::surface` (o voxel.rs de Saturno é o carregador); próximo: SurfaceCache incremental por bricks. fix/gl-probe-scopes a caminho. Mercurio removeu worktrees terminados (~35 GB).
- **Anomalia em `vrun_966123a` (16:10)**: o binário de testes da lib do sr-gpu ficou 16 min a 82 % de CPU com a GPU a 0 % (uma thread de `render::draw_uniform_tests::*` a girar: candidatos cached_mask_coverage…, compositor_batches…, compositor_dependency_checks…, mask_coverage_budget…). Nas corridas anteriores (27cb496, ba87333) o sr-gpu completou em minutos; a diferença de código é só sr-sim/sr-eval (Netuno). Matei o binário (a corrida continua nos outros binários com --no-fail-fast; a fila GPU de Mercurio segue). A seguir: repetir só esse binário com --test-threads=1 e timeout por teste para identificar o teste, e sr-3d isolado para confirmar as 19 falhas como ENOSPC. Sem push até esclarecer.
- **P5: política de overflow/fragmentMinCells (Netuno, 1766486, 16:08)**: semântica do corte (pó abaixo de min_cells; até max_fragments corpos; Overflow::Error nomeado ou Overflow::Dust com empate à primeira); API `fracture(occupancy, Partition, &FracturePolicy, size, density, ppm) → Fractured{source, pieces, piece_ids, dust, graph}`. **Decisão minha contra o compromisso**: a fonte não pode ficar mais leve antes de partir; invariante = fragmentos + pó = fonte; o momento do pó no instante da fratura é contado como perdido (contador e registo de troca), nunca pré-subtraído; teste red-first da trajetória pré-fratura com a massa inteira e da soma de momentos + perdido = fonte a 1e-9. P6 (cache de revisões) autorizado a seguir, com oráculo antes do código.
- **Saturno (16:09)**: feat/voxel-assets-eval = c2d8024 (ordem stat/limite → cache → scan → bytes com RED mostrado; stat do handle antes de ler; dependências ausentes registadas (present:false) e teste do .mtl criado; hash do mesh em streaming; pico dito (~3× ficheiro, ~4× fromMesh + importador); buraco tamanho+mtime quantificado; cópia no span máximo testada; vox_materials_hash.py com .get) e feat/pieces = 23fe0a6 (Edge::new → Result, centroid → Option, vencedores afirmados nos extremos, z do anel). Lidos por mim, aceites; em fila para merge. Netuno avisado da mudança de API. Saturno passa ao solver do blast com o oráculo 6.
- **Revisão de be86475..f00dddd + P5 (16:12): sem bloqueio**; confirmados integrais exatos (V/20, covariância, m a²/6, faces do hull fundidas, soldadura com −0,0), caminho de massa exato das peças e fonte, determinismo, frase dos hashes dourados. Itens para o tip do pó/massa inteira: teste de fecho por casca (duas caixas, uma enrolada para dentro, passam com V1−V2; doc sobre-afirma); sólidos partilhando só uma aresta recusados (dizer); flip por triângulo no leque do hull redundante e frágil; teste da carga não era vermelho contra o código anterior a 66efbd6; teste do contrato do driver depende do log de frames (afirmar o caminho); ledger "1,4×" → 1,19 aqui e 2× em geral. P5: "ocupam o lugar da fonte" não testado (sem spin: dar spin e afirmar COM e v + ω×d), teste de ordem exercita Occupancy e `graph == partition(...)` é circular, sem replay a quatro caminhos de fragmentos da função, erros só com "piece", sem entrada no ledger. Ordem de merge: f00dddd + pieces 23fe0a6 + V.1b c2d8024 na próxima janela; P5 depois.
- **`vrun_966123a` comprometida pelo disco cheio (16:12)**: além das 19 falhas de sr-3d, o passo de build GPU falhou a compilar sr-eval (lib) no mesmo intervalo (recompilado depois: sr-gpu 480/0 sem o binário da lib que matei; varreduras 38/0) e o resumo da CLI saiu vazio. Conclusão: a corrida não serve de registo; repete-se inteira no próximo head. Em paralelo: sr-3d isolado (fila de build) e o binário da lib do sr-gpu sozinho com --test-threads=1 e timeout 600 (fila GPU) para nomear o teste que girou.
- **Binário da lib do sr-gpu sozinho (16:12)**: 48/48 em 2,79 s com --test-threads=1 na NVIDIA; o giro de 16 min foi transitório (corrida paralela durante o disco cheio), não regressão de código. Fica como entrada de ambiente; a corrida completa seguinte repete-o em paralelo como sempre.
- **sr-3d isolado: 148/0 (16:14)**: as 19 falhas eram ENOSPC. **Merges (16:18–16:19)**: feat/voxel-physics f00dddd (conflito no ledger, fim da lista: ambas as entradas mantidas) = b21397b; feat/pieces 23fe0a6 = 030c5b1; feat/voxel-assets-eval c2d8024 (conflito em tests/voxels/main.rs: ambos os `mod`) = **main 1104747** (16 commits sobre 966123a; ledger 60; fmt, hygiene, mensagens ok). Erro de processo meu, duas vezes: encadeei merge + lançamento da verificação no mesmo comando e lancei sobre árvore em conflito; ambas mortas e descartadas; regra em memória (merge, verificar árvore limpa, só depois lançar). `vrun_1104747` lançada 16:20 em árvore limpa; push depois dela.
- **Mercurio (16:22)**: fix/gl-probe-scopes = f15ec4c (probe() despeja sempre os dois escopos); feat/foam-albedo = 36dcb43 (14 commits sobre 966123a; código verificado = 0ed02a5): identidade dos sete quadros, sonda hero c8a66483…, hero com albedo sem traçadores = hero sem o atributo (1f560a98… a 640×360, 3,17 vs 3,14 s); sr-gpu 542/0, CLI 68, scene-render 16, CPU 805; variância medida a f=0,5 e 8 spp: desvio da luminância 0,0354 (0,345 da média) vs 0,0061 (0,111) da mistura contínua antiga; com denoiser 0,0118 vs 0; média denoised 0,0946 vs 0,0996 (~5 % abaixo). Pergunta minha: avaliar ambos os lóbulos na NEE e sortear só a continuação (custo? obstáculo na transmissão da NEE pela água?); se barato, commit seguinte com o teste de variância; senão, limitação registada. V.3: + a5b5028 SurfaceCache (remalha só os planos dos bricks alterados: 51 de 195 planos num corte de k bricks; igualdade após 60 edições aleatórias; remalha completa por lineage/revisão/classes; orçamento limpa o cache). Revisão do foam a correr; merges após `vrun_1104747`.
- **Variância do foam (decisão 16:30)**: NEE de dois lóbulos custa 1–3 % mas só reduz a variância da luz analítica (o domo chega pela continuação); a estratificação do sorteio do lóbulo no primeiro hit (fract(u_pixel + s·0,618…) < f) custa zero e remove a variância binomial da superfície primária. Ordem aprovada: (1) estratificação com o teste de variância, identidade f=0/f=1 e ausência de padrão espacial (u_pixel decorrelacionado entre vizinhos); (2) NEE de dois lóbulos só se sobrar variância relevante com sol. Commits próprios depois do merge do foam.
- **Revisão de feat/foam-albedo 36dcb43 (16:33)**: modelo confirmado (sem rnd() a f=0, estimador de uma amostra sem fator duplo, sombra (1−f), guia médio, energia, contagens, OCN14, tabelas). **Defeitos** (antes do merge): 80 B/vértice fica abaixo do pior caso real (90–140 com HashMap + Vec por bin; medir ou grelha densa); água metálica/unlit: a SREP diz sem mistura, o código mistura (metálica) ou calcula e paga a cobertura e escreve o guia sem desenhar (unlit); emissão somada em amostras de espuma. Qualidade: sombra refratada só a f=0/1; recusa do raster comparada com background[0] e spray ainda desenhado; documentos só com água opaca (variante água+espuma nunca alcançada por documento). Nits: maxWork próprio (até 2×), OCN14 em particles, variância como teste GPU #[ignore]. Enviado a Mercurio.
- **P5 entregue num só tip (Netuno, 83bd2eb, 16:35; contém main 1104747)**: 1766486 (política minCells/maxFragments/overflow), 6e76652 (pó no mundo, red-first em sr-sim: `Fracture3::dust = Option<Dust3>`; fragmentos + pó = fonte em massa e tensor; `World3::fracture_lost(evento) → {momentum, angular_momentum}` no State, reposto com checkpoints; testes: fonte com pó = corpo inteiro até partir (1e-9), fragmentos + lost = momento linear e angular da fonte a 1e-9 com fonte a mover e a rodar, divisor do empurrão radial corrigido (4793,75 vs 4800), lost igual em mundo fresco e pedido mais tarde), 91c35aa (A1–A6: cascas com sentidos diferentes recusadas, cavidades recusadas e ditas; anel do casco como o Parry dá; `voxel_cells_held`; contrato com checkpoint_restores == 0; 1,17×), 30d47ab (merge de 1104747), 83bd2eb (B1–B5: fonte com spin e peças a v + ω×d; ordens de sementes sem empate e grafo fixado à mão com 36 faces; quatro caminhos com lost igual; mensagens; ledger). `Fractured{source, pieces, piece_ids, dust, dust_body, graph}`. Honestidade: testes de eval reescritos com a implementação (vermelhos em sr-sim); teste das cascas vermelho só após mudar o caso. Revisão independente a correr; merge depois dela e de `vrun_1104747`. P6: oráculo antes do código.
- **P6 (cache de derivados, Netuno) — oráculo aprovado (16:36)**: `sr_eval::voxel_cache::DerivedCache` pura, chave por conteúdo (fingerprint, tamanho da célula, densidade, ppm), não por revision; LRU por contador de acessos; orçamento em bytes; contadores computed/hits/evicted; 8 oráculos (1 cálculo por conteúdo, recalcula só o corpo mudado, outra história com o mesmo conteúdo acerta, voltar atrás acerta ou recalcula com bits iguais incluindo orçamento 0, chaves distintas por tamanho/densidade/ppm, orçamento nunca excedido e expulsão determinística, erros não guardados, sonda de custo fingerprint vs Moments vs components a 1e6 células). Exigência minha: fingerprint ≥ 128 bits ou verificação do conteúdo no acerto (um acerto falso é física errada em silêncio); oráculo 9 com colisão forçada; oráculo 6 com dois Programs intercalados.
- **Revisão de P5 83bd2eb (16:40)**: confirmados Dust3/FractureLost3, tensor da fonte = fragmentos + pó, lost angular em torno do COM da fonte (órbita + spin), identidade sem pó, política, determinismo. **Defeito** (antes do merge): com empurrão radial e pó, as direções são medidas do COM da fonte e a média removida só sobre os fragmentos, alterando o momento angular total em m_d (c_d − C) × média; os testes não apanham (impulso 0 ou centros colineares); correção com direções a partir do COM dos fragmentos quando há pó. Qualidade: "no State, reposto com checkpoints" não provado (fracture_lost == None após repor para antes; restores > 0); inércia do dust_body nunca verificada e tudo a ppm 1; B1 sem pó; A3 não falha num revert; mensagens por contains; capacity == len depende do alocador. Nit: regra A1 é de sinal, não de contenção (caixas aninhadas aceites; cavidade a tocar aceite; cavidade disjunta recusada): doc exato. Enviado a Netuno; P5 merge depois do tip.
- **Verificação de `1104747` limpa** (16:20–16:42): sr-sim 448, sr-eval 531, sr-model 127, sr-3d 167, varreduras 38, sr-gpu 528/0, CLI 84/0, PROBE_MATCH, clippy, fmt, hygiene. **Push a origin**: f6750db..1104747 (todas as entregas desde o push anterior: tips 3–5 de Netuno, V.1 corrigido + walk fix + V.1b, pieces, follow ledger, inércia exata em toda fratura).
- **fix/gl-probe-scopes f15ec4c integrado = main df9c3cd (16:45)**: probe() despeja sempre os dois escopos (4 linhas, lido por mim). Verificação em lote com os próximos merges.
- **3.2 pyroBlast entregue (Saturno, feat/pyro-blast rebaseado sobre 1104747: 802fd34 sedov, 2903096 êmbolo, 74f55af esquema PYC5/PYC6 + corpus 385, 83529f1 avaliador + SREP + marco; 16:46)**: ξ(1,4)=1,0328 e ξ(5/3)=1,1517 por integração das EDO (python a 2e-6; massa varrida 1/3; p(0)/p_choque 0,3062); ar deslocado = volume varrido (fluxo de saída a 1e-6; esfera coberta a 5 % com 16 células de raio); escoamento potencial a 15 % a 1,25–1,5 raios (43 % a 2 raios, dito); energia cinética ρQ²/(8πR)(1/5+1−R/a) a 0,9 %/0,55 % e em joules a ppm 37 a 1e-6; Rankine–Hugoniot em ordem de grandeza; identidade sem blast (456 testes do solver); **oráculo 6**: puff deslocado pelo volume varrido a uma célula (2 % a h=0,25); CFL 3 e 6 não saem do domínio (razões 1,026 e 1,012, sobre-desloca); **a massa de fumo NÃO se conserva a 1e-6** (3,6 % a h=0,5; 0,74 % a h=0,25): interpolação não conservativa do semi-lagrangiano, converge; teste afirma < 1 % a h=0,25; é limite do solver inteiro, a registar assim na SREP. Condições: tabela de tempos e E* ∝ dt³ (1,4e12 J a 1/24 s; 1,9e10 J a 1/100 s), sem sub-passos; PYC5 + erro do solver + erro do avaliador para escala anisotrópica; unidades com testes a ppm 1/100/37. Verificação local limpa (sr-model 128, sr-sim 456, sr-eval 518, clippy, fmt, corpus 385, hygiene). Revisão independente a correr (recomputar ξ e a energia cinética). **Item novo de Fase 3**: advecção conservativa ou correção de massa por passo no pyro (oráculo: o teste do puff a 1e-6). Saturno segue para a nota de desenho de 3.4 (fratura por tensão sobre o PieceGraph).
- **Netuno (16:50)**: feat/voxel-fracture-fix 454062c (com pó as direções do empurrão saem do COM dos fragmentos; vermelho 56,30 vs 55,85; sem pó bits iguais; empurrão por contacto partilha as direções, sem teste próprio, dito) + itens de teste (lost reposto com restores > 0 e None antes da fratura; tensor do pó vs forma fechada m(b²+c²)/12 a ppm 1 e 100; B1 com pó; A3 declarado como fixação; mensagem inteira; capacity ≤ len + len/16; doc da regra de sinais). **P6** c4d13d1 (tip 28f1dcf): `DerivedCache` com chave (fingerprint 64 bits, nº células, nº bricks, tamanho, densidade, ppm) e bricks guardados por entrada, comparados byte a byte no acerto (colisão forçada testada); LRU determinística; orçamento; 9 testes; sonda com 968 575 células: fingerprint 1,26 ms, moments 38 ms, body 32 ms, components 386 ms, acerto 1,3 ms; 28 B/célula retidos. Honesto: testes da cache passaram à primeira (stub). Revisão independente a correr; merge de 28f1dcf após ela e a do blast.
- **Nota de ligação à cena (Netuno, 16:55; fora do repo)**: shape_for → Shape3::Voxels; slots só em donos com crater ou fracture; Driver3::voxel_cut com kernel conservador bulking 1 + crater_cut; fracture com voxels::fracture + Fracture3{dust}; `Frame3::voxel_revision` por corpo e `World3::voxel_cells_at(corpo, revisão)` para o renderizador (aprovados; a revisão exposta é a que a SurfaceCache de Mercurio espera); atributos pedidos ao esquema (rigidBody@shape="voxels", density obrigatória, maxFragments 1..4096 def. 64, fragmentMinCells, fragmentOverflow, anchor; crater em dono voxel CRT13+; fracture@partition voronoi|planes|labels; burst por células). Decisões: curve/start/end em crater de dono voxel = erro; interiorMaterial em fratura de células = erro por agora, caminho na SREP. Saturno responde ao mapa objeto↔célula e escreve o esquema (feat/voxel-scene-schema) depois da nota de 3.4.
- **P5 + correção + P6 integrados = main 9e78d4a (16:52)**: revisão de 454062c e c4d13d1 sem defeitos de correção (derivação do momento angular confirmada; cache nunca dá resposta errada; LRU sem empates; testes rejeitam sempre-recomputar, obsoleto, FIFO e MRU); nits para seguimento (docs do empurrão; radial_impulse dividido pela massa da fonte com pó; orçamento da cache por capacidade; snapshot antes de copiar). Ledger 61. Verificação em lote com o blast.
- **Revisão do pyroBlast (16:53)**: tudo recomputado e confirmado (ξ0 pela forma fechada de L&L a 1e-6 do módulo; energia cinética; tabela; 272/385; PYC5/6; identidade; unidades). **Defeitos** (antes do merge): (1) pulso que cobre a janela fica centrado na JANELA (faces p=0 + divergência uniforme), não no blast: a frase da SREP é falsa acima de ~9e7 J na hero; correção: injetar o campo de velocidades analítico do êmbolo antes da projeção; testes para janela recortada, follow, obstáculo, blast+fonte; (2) R forçado ≥ voxel_size com divergência 1: volume deslocado independente de E para E < p0·(h/(0,3·ppm))³ (1 J desloca 0,9 m³); escalar a (V1−V0)/(N h³ dt); (3) γ=1,05 dá NaN em solve; (4) E* errado nos textos (1,91e12 e 2,64e10, não 1,4e12/1,9e10); (5) limiar da hero errado (9,0e7 J, não 1,2e5). Nits: 0,3 citado (é p_s ≈ 5,8 p0), anisotropia só por normas, tolerâncias frouxas (KE < 10 %, deslocamento < 1 célula), testes por construção, Sedov re-resolvido a cada passo, 9 pulsos não 8. **3.4 fratura por tensão**: desenho de Saturno (variante A: peças como corpos desde t=0 soldadas nas juntas; tensão da reação da solda: σ_n, τ, σ_b com W exato por `face_second`; critério de tração principal ≥ strength; ordem (a,b) determinística; oráculos viga encastrada com P* = strength·W/L, coluna comprimida, bits iguais, conservação após rotura, repouso 2 s; spike de custo ≤ 5 ms a 256 peças; variante B se falhar) aprovado; blast primeiro.
- **Netuno feat/voxel-frames cf2f6c7 (16:58)**: fb57b0f (API do lado do frame: `Frame3::voxel_revision` por corpo (Some(n) cortes; Some(0) asset; None não cortável/slot vazio; um corte = uma edição; peça nasce com Some(1)); `World3::voxel_cells_at(corpo, revisão)`; `World3::voxel_bricks_changed(corpo, de, para)` simétrico com esvaziados; orçamento da história 256 MiB, nunca descarta a revisão atual; 5 testes vermelhos contra stubs; Mercurio escolheu bricks_changed por corpo e acrescenta SurfaceCache::update_known; o Occupancy com paleta por (corpo, revisão) será feito no evaluator com a paleta do asset e da borda), cf2f6c7 (nits: cache por capacidade após shrink_to_fit; snapshot ao tamanho real calculado antes de copiar; sem empates na doc; colisão/custo para components e body; radial_impulse documentado como dividido pela massa da fonte incluindo pó (bits intactos); comentário do centro; escala guarda momento angular). Saturno respondeu o mapa objeto↔célula (canto mínimo, y para baixo, centro (k+0,5)·cellSize; anchor "base" = camada de maior y; cell_to_object/object_to_cell em sr_3d::voxel). Revisão independente a correr; merge após `vrun_9e78d4a`.
- **Resumo da faixa de voxels (Netuno, dafe57b sobre cf2f6c7, 17:12)**: entrada nova no ledger (5,7 k caracteres) com SCOPE, occupancy e massa, cortes e slots, cratera em células (6460/5168/1292 e o pilar), fratura com pó (fragmentos + lost = fonte a 1e-9), cache, frames, como foi verificado (incluindo os testes que passaram logo e os defeitos das revisões), NOT WIRED (tudo o que espera o esquema), UNTESTED (chão inclinado, eixo fora do plano, empurrão por contacto com pó, momento do pó perdido, Decomposition fora de fratura, regra das cascas por sinais), COST. Lido e aceite; pedido um parágrafo factual de cinco linhas na SREP sob a tabela do roadmap apontando para a entrada.
- **Verificação de `9e78d4a` limpa** (16:53–17:12): sr-sim 456, sr-eval 552, sr-model 127, sr-3d 167, varreduras 38, sr-gpu 528/0, CLI 84/0, PROBE_MATCH, clippy, fmt, hygiene. **Push a origin** 1104747..9e78d4a (P5 com pó e correção, P6 cache, gl-probe-scopes).
- **Parágrafo da SREP sobre a física de voxels (Netuno, 52dbf47, 17:15)**: no fim de "Voxel assets and objects", cinco frases factuais (ocupação e massa exata; cortes e slots com identidade das quatro maneiras; cratera 6460/6352; fratura com pó e momento perdido; cache e frames) + "não ligado a nenhuma cena; por testar chão inclinado e eixo fora dos planos", remetendo para a entrada do ledger pelo nome. Aceite; feat/voxel-frames (cf2f6c7, dafe57b, 52dbf47) integra após a revisão da API de frames.
- **Revisão da API de frames (fb57b0f/cf2f6c7), 17:18**: confirmados invariante da revisão por construção, uma edição por corte, regra "nunca a atual", diferença simétrica de bricks, história fora do State refeita por replay, nits de cf2f6c7. **Defeitos**: (1) revisão descartada pelo orçamento é irrecuperável para um frame servido do log (o log nunca regrava as células; orçamentos do log e da história separados; overwrite não refresca a ordem): refazer o frame no miss e regravar; (2) contrato "ordem do scan" vs ordem x-major do sort: alinhar ou corrigir doc, teste contra Occupancy::cells(). Qualidade: voxel_revision nunca verificado no caminho de replay (same() só compara enabled/pose/velocidade; frase do ledger falsa até lá); sem testes de orçamento 0 e de revisão posterior após replay. Nits: carga da história só conta células; brick 8 literal em sr-sim (const nomeada + teste de igualdade em sr-eval); carga de components conta headers duas vezes. Enviado a Netuno; merge de feat/voxel-frames após o tip.
- **Foam corrigido (Mercurio, ae0da55, 17:22; 17 commits sobre 966123a)**: bfce6ef grelha densa de bins com memória exata 4·(quadrados+1) + 16·vértices verificada antes de alocar (orçamento exato passa, um byte a menos recusa; bits da cobertura iguais); 823a3d1 água metálica aceita (0,1926 vs 0,1928), unlit/emissiva recusadas com erro nomeado + W08 e corpus; ae0da55 sombra refratada a f=0,5 (0,5394/0,2985/0,0003; degrau e quadrado excluídos), recusa do raster com todos os pixels contra #101020 linear, documento com água transmissiva (30,07/295,12/559,91; sem traçadores bit a bit), variância como teste GPU #[ignore]. Verificação local: CPU 807, sr-gpu 544/0, CLI 68, scene-render 16, identidade dos sete quadros e sonda c8a66483…. Revisão curta a correr; depois merge, verificação, push. Estratificação em feat/foam-stratified. V.3 com 11 commits (renderer: object3D voxels desenhado por raster e path tracer sem tocar shader; sonda: 6 quads 18 ms, casca 157 716 quads 61 ms, corte de 904 células remalha 59 planos em 4 ms vs 60 ms).
- **Frames corrigidos e integrados (Netuno, ddcd109; 17:28)**: frame_at verifica que a história tem as revisões do frame do log e, se não, descarta-o e refaz (teste vermelho: mundo até 1 s, história cortada, frame_at(0.1), voxel_cells_at(0,0) era None); frame refeito de checkpoint regista as células (record_current_voxel_cells); sobrescrever refresca a ordem (por leitura, dito no ledger); contrato da ordem corrigido na doc (chave x primeiro, não scan; teste fixa-o; Mercurio avisado); same() lê voxel_revision (caminho replayed coberto); orçamento 0 e revisão posterior após replay testados; carga = capacidade + 96 B por entrada; VOXEL_BRICK nomeada com teste de igualdade em sr-eval. Lido por mim (diff do frame_at e da carga). Merge feat/voxel-frames em main; verificação em lote com o foam.
- **Revisão dos commits de correção do foam (17:30)**: grelha densa, metálica, recusas, W08, pixel a pixel, razão da transmissiva, SREP: confirmados. Dois defeitos baixos (o avaliador calcula e cobra a cobertura mesmo quando a água vai ser recusada; W08 não bate com o critério do renderizador e não vê props animadas) e um teste de identidade errado (compara dois documentos com foamMode entre si, não contra a água sem o atributo); nits (banda ±15 % com 4 % de folga; ruído a f=0/1 sem comparação; counts.clone() fora da fórmula; posição ±inf satura; textos). Enviado a Mercurio para um tip pequeno; o foam integra depois. `vrun_4c5159b` (frames) lançada 17:26; push depois.
- **pyroBlast corrigido (Saturno, afefb5c sobre 9e78d4a; 17:32)**: d0a44a7/afefb5c: `inject_piston` põe o campo analítico do êmbolo na velocidade antes da projeção (interior (x−c)·d/3, exterior Q/(4πr²)); teste com blast a 1 m da face: o ar entre o meio da janela e o blast vai para −x e o interior é linear (10 %); obstáculo na esfera (fluxo = varrido/dt a 1e-4), fonte com expansão no mesmo passo (soma exata), follow sem mover (bits iguais); volume varrido repartido pelas N células não sólidas (1 J → 1,1e-6 m³; fluxo a 1e-6 em 14 passos somando V(R_max)); γ ≥ 1,1 no solver, Blast::new, XSD, regras, SREP (1,05 recusado; solve_cached); E* = 1,91e12 / 2,64e10 J; limiar da janela E ≥ p0 (L/0,6)³ = 9,0e7 J; 0,3 derivado (p_s/p0 = 11,85 ξ0⁵/(γ+1) = 5,8 para ar); uniform_length (comprimentos iguais E ângulos retos) para blast e crater com teste de cisalhamento; tolerâncias KE 2 %, deslocamento 5/3 %, avaliador ±0,05 m; volume em vários passos; campos privados. Números novos: fumo a 6 m move 0,984/0,990 do exato; massa muda 3,6 %/0,70 %. Verificação local limpa (sr-model 128, sr-sim 477, sr-eval 556, corpus 385). Revisão curta a correr; merge após `vrun_4c5159b`. Saturno passa ao esquema de voxels e depois ao spike de 3.4.
- **Revisão das correções do blast (17:40; modelo numpy de inject_piston + projeção em piston.py)**: confirmados alvo absoluto, boundaries após injeção, centro no mundo, Q exato, E*, limiares, frente, 1 J, 11,85, γ, uniform_length. **Defeitos**: (1) com a esfera recortada d = Q/(N h³) sobre as células cobertas dentro da janela empurra todo o volume por parte da esfera (teste da janela 1,67×; janela dentro da esfera 5,84×; canto ~8×); correção d = Q/max(V(R1), N_total h³); (2) o campo de velocidades injetado persiste após o êmbolo parar (∇H solenoidal com valores de fronteira não nulos nunca é removido pela projeção com p=0; 50–200 % do pulso fica; o esquema antigo deixava 0): injetar como delta e subtrair no passo seguinte; teste de decaimento. Qualidade: linearidade ±16,7 %; cisalhamento não testado (mapa inverso); sem teste de crater; solve_cached com corrida. Nits de texto. Enviado a Saturno; merge do blast espera.
- **Verificação de `4c5159b` limpa** (17:26–17:44): sr-sim 464, sr-eval 555, sr-model 127, sr-3d 167, varreduras 38, sr-gpu 528/0, CLI 84/0, PROBE_MATCH, clippy, fmt, hygiene. **Push a origin** 9e78d4a..4c5159b (API de frames dos corpos de voxels, resumo da faixa no ledger e na SREP).
- **Foam, tip pequeno (Mercurio, 872ff3f, 17:48)**: `sr_model::foam::water_takes_foam` partilhado (opaca, não unlit, não brilha: força > 0 e cor não preta; grafias de preto; qualquer outra cor/token brilha, errando para a recusa); o avaliador só faz a cobertura se o material aceita E há câmara pathtrace (teste: água lit a 601×601 falha no orçamento, os 5 casos recusáveis saem sem cobertura); W08 com a mesma função, props animadas só o renderizador vê (dito); teste transmissivo a f=0 contra o MESMO documento sem foamMode bit a bit (2 albedos × 2 rugosidades); ruído a f=0 vs água sem mistura na mesma corrida; banda ±20 %; "not drawn" afirmado; counts.clone() eliminado; posição infinita/NaN não cobre; SREP corrigida. Lido por mim; merge quando chegarem os números da verificação completa de Mercurio. feat/foam-stratified já tem os testes (cv < 0,25 a 8 spp; |ρ| vizinhos < 0,15; média a 128 spp a 5 %).
- **Foam integrado = main a590300 (19:22, após pausa)**: verificação de Mercurio em 872ff3f limpa (CPU 810, sr-gpu 544/0 +1 ignorado, CLI 68, scene-render 16, identidade dos sete quadros e sonda c8a66483…); merge sem conflitos; `vrun_a590300` lançada; push depois. **Estratificação** feat/foam-stratified 6493933 (lóbulo do primeiro hit por fract(hash do pixel + amostra·0,618…); desvio 0,0155 (cv 0,156) vs 0,0354 aleatório e 0,0061 da mistura antiga; com denoiser 0,0003 vs 0,0118; média 0,0984 vs 0,0996 (1,2 %); correlação de vizinhos ≤ 0,014; f=0 bit a bit; teste #[ignore] com limites cv ≤ 0,2, denoiser ≤ 0,02, média a 3 %): a integrar a seguir. NEE de dois lóbulos: registada como medida futura (cena com sol e espuma albedo).
- **Saturno, ponto de situação (19:38)**: feat/pyro-blast rebaseado em a590300, tip 48b8900 (detalhes em 8c01652): (1a) volume repartido pelas células da esfera INTEIRA (rede infinita por colunas, menos sólidas na janela); janela de 16 m com blast a 1 m da face dá u_x(0) do espaço livre a 5 %, u(4)/u(2) = 0,6 ± 0,03, divergência da esfera a 10 %; (1b) **decisão de desenho diferente da minha proposta**: o escoamento do êmbolo é calculado à parte (projeção própria, linear) e transporta densidade e temperatura UMA vez no passo (clone com a velocidade do blast); a velocidade do fumo fica bit a bit igual ao caso sem blast em cada passo (14 passos com turbulência); custo: segunda projeção e clone só nos passos com blast; consequência dita: o momento do ar não é transportado. Nits: cisalhamento real (pai 45°, filho 0,8/1,5118; produto escalar 0,5625), crater com esticão em z e cisalhamento, entry().or_insert, 9,0, cantos 4,66e8 J, 0,6 ± 0,03, avaliador fixa o medido (1,19 m; físico 1,06). Verificação local: sr-model 133, sr-sim 487, sr-eval 575, corpus 392, clippy, fmt, hygiene. Revisão independente a correr. Esquema: wip 3a881e4 + cell_to_object/object_to_cell (d155e03); faltam corpus, testes de regras, SREP, ledger; ETA próxima ronda. Spike 3.4 não começado.
- **Revisão de 48b8900 (19:45; piston2.py)**: esquema à parte aceite (sem velocidade residual é correto em escoamento potencial). Confirmados contagem exata por colunas, sólidos, sem overflow, projeção com Err, ordem, 3,6/0,70 % inalterados. **Defeitos**: dissipação e arrefecimento aplicados duas vezes nos passos com blast (a advecção do blast usa o mesmo advect; todos os testes a 0); velocidade de colisor em movimento contada duas vezes (boundaries nas faces sólidas da projeção só-blast). Qualidade: identidade bit a bit passa por construção (flutuabilidade 0); tolerâncias 5 %/±0,03/10 % contra 0,99988/0,6004/0,998 medidos. Nits: lattice_count vs in_sphere na fronteira; profile sobrescrito; blast_flow não limpo no follow; custo ~1,6 GB transitórios a 256³; SREP:908 limites reais (sem impulso ao fumo, pluma posterior não desviada, divisão de primeira ordem, sem após-fluxo). Enviado a Saturno; merge do blast espera.
- **Saturno (19:50)**: blast tip 45dcad1 (decay/cooling uma vez: Spec do blast com dissipation=0 e cooling=0, teste a 1000/s; faces sólidas do fluxo do blast a velocidade zero, teste com caixa a 3 m/s; identidade afirma que o blast fez algo + caso com flutuabilidade; tolerâncias 2 %/1,5 %/±0,01/2 %; lattice_count com o mesmo teste de centro (400 casos vs força bruta); profile.blast; blast_flow limpo em shift_window; orçamento 222/288 dito; SREP com ordem, custo e limites reais). Esquema tip 29ab659 (4c459b1 cell_to_object/object_to_cell/file_key; 29ab659 VOX8–VOX14, CRT5+voxels, CRT13–CRT16, FRX3/FRX8–FRX12; Schematron e Rust concordam em 20 inválidos + 6 válidos; corpus 412; 5 testes de regras; SREP "Bodies of cells in the scene" + inventário, 287 asserções; marco). Verificações locais limpas. Duas revisões a correr; merges após `vrun_a590300` com os conflitos esperados (ledger; contagem 289). Spike de 3.4 começa.
- **3.4 spike (Saturno, e0a5d86 em feat/fracture-stress, 19:59)**: variante A (soldas do Rapier) passa em repouso (deriva 0,115/0,309/0,851 mm a 8/64/256 peças, sem fluência) e em voo (momento a 1e-14), mas FALHA no impacto a 100 m/s com o passo do filme (8 peças 8,67e7 J, 64 peças 2,02e8 J = 2,1× a energia inicial, 256 peças 4,19e7 J; a 960 passos/s passa mas custa 4×; 6,1 ms/passo a 256 peças, acima dos 5 ms). **Decisão: variante B** — um só corpo rígido até o critério disparar; cargas internas por junta por Newton–Euler sobre os impulsos de contacto e a aceleração do corpo (exato em árvores; em ciclos, resultante do corte mínimo distribuída por área, dito); critério de tração principal ≥ strength; ordem determinística; as juntas rotas definem a partição para voxels::fracture (pó e overflow pela política); conservação por construção; limites: rígido até partir (sem ondas de tensão), cargas só em passos com contacto/aceleração; oráculos: viga com P* = strength·W/L exato, coluna, bits iguais, identidade sem @mode, custo a 256 peças, anel de 4 (regra de área). e0a5d86 fica como registo do spike no ledger.
- **Revisão de 45dcad1 (20:02): sem defeitos** (decay uma vez com dt=5e-4; Spec clonado inofensivo; faces sólidas a zero sem subtração dupla; lattice_count = in_sphere com força bruta independente, fórmula antiga falharia 19/480). Nits: duas asserções do teste do colisor não discriminam; identidade não cobre passos sem pulso; memória real ~245–260 B/célula (blast_flow anterior vivo, at_rest duplicado, clones redundantes); follow() não limpa blast_flow. Blast entra em main após `vrun_a590300`.
- **Plano de ligação à cena (Netuno, 20:01; feat/voxel-scene-wiring sobre 29ab659) aprovado**: W1 corpo (massa via choque frontal = N·density·volume a 1e-6; propriedades exatas; identidade sem voxels), W2 corte (solo .srvol 120×40×120 escrito pelo teste; voxel_revision Some(1); células = asset − removidas + borda com contagens fixadas e iguais às de crater_cut direto; peça num slot; quatro maneiras; 2.º impacto não corta), W3 fratura (fragmentos + pó = asset; momento com fracture_lost a 1e-9; overflow=error), W4 saída (Arc<Occupancy> por (corpo, revisão) com paleta do asset e da borda; births do burst = células lançadas), W5 identidade e limites (cache física de mundo com corpos de células recusada com mensagem). Decisões de Netuno aceites: eixo da cratera = normal do impacto; corte instantâneo; slots só para crater; planos arredondados ao lado da célula fora de meias células. Adições minhas: a recusa da cache nomeia o atributo e entra em NOT WIRED; W2 afirma voxel_bricks_changed(0→1) = bricks tocados.
- **Revisão do esquema de voxels 29ab659 (20:10)**: paridade Rust/Schematron, corpus, XSD, referencial e contagens confirmados (oráculo lxml corrido numa cópia + 28 docs extra). **Defeitos**: VOX13 chaveada em $cells enquanto as outras são por primitiva (dono voxels com shape trimesh/box + crater/fracture e escala não uniforme é aceite sem significado): exigir shape voxels/auto com crater ou fracture; os documentos válidos novos são mal avaliados hoje (esfera com massa 1; burst count 0 falha; fratura "no solid geometry") e backends.rs do sr-gpu avalia todos os válidos: decisão = recusa do avaliador pelo nome até à ligação de Netuno + lista explícita em backends.rs, que W1–W3 vão retirando. Qualidade: loop do teste de regras só afirma !is_empty() (CRT5 dispara sempre). Nits: FRX11 só exclui NaN; "+2" NaN; escala ancestral/animada; angle/angleSpread no burst de células; CRT14 start/end nunca só; docs desatualizados; object_to_cell em faces com 0,3/0,1. Enviado a Saturno (commits sobre 29ab659) e a Netuno (W1–W3 retiram a recusa).
- **Esquema corrigido (Saturno, dc9e034 sobre 29ab659; 20:20)**: b0856b2 VOX15 (dono que parte tem as células como colisor), 7ff52fe E23 "bodies of cells are not evaluated yet by this build" por objeto + lista explícita dos 6 documentos recusados em backends.rs (falha nos dois sentidos; antes o erro era engolido por .ok()?), 783224e códigos por caso, 4fb67e6 FRX11 finito e normal não nula, ff053f4 "+2", 4ea3b74 CRT14 só curve + CRT17 (angle/angleSpread no burst de células), 42f0aad object_to_cell com tolerância de 4 ulps nas faces, dc9e034 docs (anchor base por defeito; tabela VOX1–15; limite de VOX13 dito; ledger sem re-indentação; 289/418). Verificação local limpa menos o grupo backends do sr-gpu (na fila GPU). Slip declarado: um `git stash pop` nu numa experiência, lista vazia antes e depois, sem dano. Merge em lote após o resultado do backends e `vrun_a590300`.
- **Bloqueio dos testes da lib do sr-gpu, segunda vez (20:27)**: o mesmo binário (sr_gpu-b6df95da…) ficou 46 min parado (GPU 0 %, todas as threads em futex_wait, duas em futex_lock_pi, 5 devices Vulkan abertos por fx/output/render/raster) na `vrun_a590300`; às 16:08 girara 16 min; sozinho com --test-threads=1 passa 48/48 em 2,8 s. Não é ENOSPC: é um deadlock dos testes paralelos (provável inversão entre o mutex de criação do device e poll/map do wgpu). Dump de threads guardado. Ações: binário morto (a corrida continua); o script de verificação passa a correr a lib do sr-gpu com --test-threads=1 e os testes de integração em paralelo; Mercurio encarregado da causa raiz e correção estrutural red-first, com prioridade sobre V.3.
- **`vrun_a590300` concluída (20:32)**: sr-sim 464, sr-eval 570, sr-model 132, sr-3d 167, varreduras 38, sr-gpu 492/0 (sem a lib, morta) + lib 52/52 em 3,9 s com --test-threads=1 (= 544), CLI 84/0, PROBE_MATCH, clippy, fmt, hygiene. **Push recusado**: o usuário fez push direto a origin de dois commits (c126a7e "Fix temporal frame invalidation and output-layer delivery": sr-deliver pipeline/overlay/áudio, testes da CLI; 529cd89 "Simplify upstream boolean guards for Clippy": occupancy.rs, voxel_crater.rs, voxels.rs, granular.rs, safe_audit). Integrados em main por merge sem conflito.
- **Merges em lote (20:36)**: feat/foam-stratified 6493933, feat/pyro-blast 45dcad1 + nits 4b50406 (blast_flow descartado no início do passo e no follow; passo sem pulso após R_max sem fluxo; pico medido por alocador 206 B/célula com blast, 141 sem; 80 testes do pyro), feat/voxel-scene-schema 0c8a6ac (lista de 7 documentos recusados num teste próprio sem adaptador; o grupo backends só tem GL, que salta aqui). Verificação única a seguir; push depois.
- **Merge do esquema com conflitos resolvidos = main c829ca3 (20:37)**: SREP (contagem combinada: 291 asserções, 111 cinemáticas, 44 não vendidas incluindo PYC5/PYC6; recontado no .sch fundido), build_corpus.py (ambas as listas), ledger (ambas as entradas); corpus 423 com oráculo lxml a concordar; fmt e hygiene limpos. `vrun_c829ca3` lançada 20:37 (a primeira tentativa não arrancou: guarda do comando encadeado; regra: lançar sempre num comando só) (lib do sr-gpu com --test-threads=1); push depois. Netuno avisado para fundir main antes de W2.
- **Verificação de `c829ca3` (20:37–21:04)**: sr-sim 495, sr-eval 578, sr-model 138/1 (ten_thousand_nodes_load_under_200_ms, teste de tempo sob carga 15–17; passa sozinho em 0,35 s), sr-3d 173, varreduras 38, **sr-gpu 600/0** (lib com --test-threads=1), CLI 87/0, PROBE_MATCH, clippy, fmt, hygiene. **Push a origin** 529cd89..c829ca3 (foam albedo + estratificação, pyroBlast, esquema de voxels, merge dos dois commits do usuário).
- **Deadlock dos testes da lib do sr-gpu: causa e correção (Mercurio, d04584d; 21:13)**: reproduzido sob carga (3 bloqueios em 77 execuções com 16 threads; 0 sem carga); dump com strace: uma thread no mutex CREATION, duas em FUTEX_LOCK_PI no mesmo mutex PI do driver NVIDIA, 7 devices abertos (sete testes da lib abriam cada um o seu); o CREATION só serializa a criação, não o uso simultâneo de vários devices no mesmo processo. Correção: `test_gpu()` (cfg(test), OnceLock) partilhado pelos 7 testes + guarda em Gpu::open que falha se um teste da unidade abrir um segundo device (6 de 52 vermelhos com os testes antigos); 100/100 sob a mesma carga. Integrado = **main b255d61**; o script de verificação volta a correr a lib em paralelo (teste real da correção); `vrun_b255d61` lançada 21:13. Item novo (não agora): o mesmo risco em produção com Gpu::open_like por trabalhador em entrega paralela (um device por processo ou uso serializado, com medição).
- **Ligação à cena W1–W5 entregue (Netuno, feat/voxel-scene-wiring 0ce4950 sobre c829ca3; 21:20)**: W1 corpo (bloco 8³ de 0,25 m a 2400 kg/m³ pesa 19 199,92 kg vs 19 200; escala 2 em x = 38 400; escala negativa nomeada), W2 corte (solo 120×40×120 + pilar; bola 90 478 kg a 86,6 m/s; revisão 0 → 1; células, peça de 136 células, paleta e bricks IGUAIS a crater_cut_of direto; 1416 amontoadas, 7080 destruídas, 5664 lançadas, 1 peça; fresco/depois/de novo iguais; 2.º impacto não corta; FrameNode::voxels com SimVoxels{grid Arc<Occupancy> com paleta, changed_bricks, steps, thrown, pieces}), **77bdada defeito do mundo**: fratura depois de a fonte rodar dava velocidades às peças com os centros de massa de corpos fora do mundo (3813 de 4800 kg·m/s em y; 6312 em z); agora usa as propriedades registadas (World3::fracture_props); nenhum teste existente mudou; W3 fratura (planos com pó, sementes, labels=material, overflow, momento das peças = do bloco a rodar), W4 burst de células (uma partícula por célula lançada; 5664), W5 cache física recusada com "remove physics@cache" e identidade sem voxels; 71f5665 remove E23 e a lista; os 3 docs válidos de fratura avaliam para "no piece of the fracture is a body" (asset de 5 células): pedido a Saturno fragmentMinCells=1 ou asset maior + teste de que todo válido avalia limpo. Revisão independente a correr; merge após ela e `vrun_b255d61`.
- **Defeito no ponto de contacto dos corpos de células (Saturno, 21:25)**: `collect_contacts` ignora a sub-pose da forma composta ((pose1·local_p1 + pose2·local_p2)/2 com local_p no referencial da célula): coluna de 0,5 × 3 m em repouso regista pontos a y = −0,808 em vez dos cantos do fundo a y ≈ 0 (até uma célula ou mais de erro); afeta impact_in_step, os relógios de impacto e o ponto/eixo da cratera do W2. Decisão: correção em fix/contact-points-of-cells (Saturno, vermelho primeiro, sub-pose nos dois lados, identidade para formas sem sub-pose); entra em main antes da ligação; Netuno funde e re-fixa as contagens do W2. 3.4 variante B: viga encastrada a 5e-4; 0,995 P* não parte, 1,005 P* parte só a raiz.
- **Revisão da ligação W1–W5 (21:30)**: confirmados unidades, contagens, planos, W4/W5, 77bdada correto (fórmula derivada; malhas recentradas no centroide, só bits baixos), determinismo. **Defeitos**: fratura de células não cria FractureNode e `apply` lê a bandeira pelo índice da lista só de malhas (documento com fratura de células antes de uma de malha mostra a de malha no instante errado); o pó de um corte de cratera nunca vira partículas (só excavation.thrown; contradiz o XSD; W4 "massa total" falha com fragmentMinCells > 1). Qualidade: nenhum teste com ppm ≠ 1; contagens do W2 só do código em teste (verificação geométrica pedida; o topo do pilar é cortado pelo teto = crista, não pelo impacto: dizer ou decidir); "2.º impacto não corta" sem 2.º projétil; banda de velocidade do W4 frouxa. Nits: paleta 1 silenciosa; fallback morto; dois predicados da recusa da cache; o renderizador ainda não lê FrameNode::voxels. Enviado a Netuno junto com a espera pela correção do ponto de contacto.
- **Correção do ponto de contacto (Saturno, c2e30e0, 21:32)**: sub-pose aplicada em cada lado que a tem; coluna de 6 células dá os 4 cantos do pé (antes y = −0,8075); barra de 8 células como guarda; prisma composto e esfera sobre malha bit-idênticos (bits registados antes da mudança); sr-sim/sr-eval verdes. **Netuno 216ffef**: os dois defeitos corrigidos (FractureNode.event; pó como partículas em repouso no referencial do dono), ppm 100, 2.º projétil (ressalta 13 m, cai na cratera a ≈4 s, revisão 1 igual), velocidade a +0,02 s, nits; **o teste geométrico independente FALHA de propósito** (110,6 m³ destruídos vs 100,9 da lei, +8 %): é o efeito do ponto de contacto errado, confirmando a suspeita; fica vermelho até c2e30e0 entrar; depois re-fixa as contagens. Regra do teto (crista) mantida por escolha dita. Ordem: push de b255d61 → merge de c2e30e0 → Netuno funde e re-fixa → merge da ligação → verificação → push.
- **Verificação de `b255d61` limpa (21:13–21:32)**: sr-sim 495, sr-eval 578, sr-model 139, sr-3d 173, varreduras 38, **sr-gpu 548/0 com a lib em paralelo** (a correção do deadlock funciona), CLI 87/0, PROBE_MATCH, clippy, fmt, hygiene. **Push a origin** c829ca3..b255d61. Merge de fix/contact-points-of-cells c2e30e0 em main a seguir (verificação em lote com a ligação de Netuno).
- **fix/contact-points-of-cells integrado = main a582d22 (21:33)**; Netuno funde, re-fixa as contagens do W2 e entrega; verificação única da ligação + correção; depois push.
- **Ligação à cena integrada = main 8f51a48 (21:44)**: Netuno b1a9d57 (contém a582d22). O teste geométrico independente encontrou um SEGUNDO defeito, do próprio wiring: o ponto de contacto de uma bola que entrou no solo no passo do contacto fica DENTRO das células (0,093 m ≈ 0,37 célula) e o kernel era feito nesse plano, cortando uma camada a mais (+8 %); c2e30e0 sozinho não mudava as contagens. Correção e5b43bc: `crater_cut_of` põe o plano onde o eixo pelo ponto encontra a superfície das células (`surface_along`, bissecção a 40 passos, alcance de 6 células, fallback dito). Contagens novas = cratera pura da lei: 6460 destruídas (100,94 m³ vs 100,92), 1292 amontoadas, 5168 lançadas, peça de 132 células; teste geométrico (volume a 3 % com o pilar retirado; alcance de cada célula) passa; dois testes de cache deixaram de partilhar um diretório temporário. `vrun_8f51a48` lançada 21:44; push depois. Mercurio avisado do FrameNode::voxels para V.3; Netuno escreve a atualização do resumo do ledger após o push.
- **Dois erros de processo meus descobertos pelos agentes (21:50)**: (1) o script de verificação só mostrava a última linha do clippy e não lia o estado: `cargo clippy --workspace --all-targets -D warnings` FALHA desde c829ca3 (teste overlay_audio.rs do commit do usuário c126a7e: lint chunks_exact_to_as_chunks) e c829ca3/b255d61 foram a push assim; corrigido o script (CLIPPY_EXIT + linhas de erro) e o lint vai a main num commit pequeno; (2) a minha resolução manual do conflito do ledger em c829ca3 corrompeu a estrutura (chaves duplicadas dentro da entrada do pyroBlast, uma entrada só com limits, duas repetidas; json.load aceitava): Netuno reparou em 1b927e4 (docs/voxel-track-summary 72da0d8, com o resumo atualizado: NOT WIRED reduzido ao renderer/srvseq/cor da célula lançada/pó de dono móvel/cache; secção "o que os oráculos de ponta a ponta acharam") e escreve a verificação estrita do ledger no hygiene. Memória atualizada.
- **Regra L do hygiene (Netuno, fix/ledger-structure-check dbe3828, contém 72da0d8; 21:55)**: check_ledger em --all e --staged: L1 chave repetida/não faz parse (object_pairs_hook), L2 name/evidence/validation/limits em cada marco, L3 nomes repetidos; 7 testes vermelhos antes; recusa o ledger de main ("the key 'name' is repeated in one object") e aceita o reparado. Entra com a correção do lint em sr-deliver após `vrun_8f51a48`; verificação; push. Netuno: próximo oráculo = cratera em chão inclinado.
- **Verificação de `8f51a48` (21:44–22:00)**: testes todos limpos (sr-sim 499, sr-eval 593, sr-model 139, sr-3d 173, varreduras 38, sr-gpu 547/0, CLI 87/0, PROBE_MATCH, fmt, hygiene) e clippy do workspace a falhar (lint de sr-deliver, conhecido). **Correções em main (22:03)**: cb47f2b (overlay_audio.rs lê os quadros com as_chunks; clippy -p sr-deliver limpo; teste passa) + merge de fix/ledger-structure-check dbe3828 = **main bc3c856** (ledger reparado, 67 marcos, parse estrito ok; regra L no hygiene com 17 testes ok; resumo da faixa atualizado). `vrun_bc3c856` lançada 22:04 com CLIPPY_EXIT no relatório; push depois.
- **Oráculo do chão inclinado (Netuno, test/sloping-ground-crater c139c36; 22:08)**: rampas a 10/20/30° (30×30 m, células de 0,25 m, 1,1 M células) com a bola ao longo da normal: FALHAVA ("the rim has room for 406 of the 2160 cells"): a normal de contacto do mundo é a da face/aresta tocada primeiro, 13–15° fora da normal da rampa numa escada de células. Correção: eixo da cratera = normal exterior da superfície (direção do centróide das células cheias no raio da crista até ao ponto), encostada ao eixo da rede abaixo de 2° (ruído ~0,1° em chão plano; sem encosto 6460 → 6466), normal de contacto como recurso. Chão plano inalterado; rampas: eixo a ≤ 0,1° da normal; volume 101,1/98,3/96,1 m³ vs lei 96,0/96,6/97,2 (+5,3/+1,7/−1,2 %, tolerância 7 % declarada). Por testar: impacto oblíquo, eixo fora do plano da rampa, chão curvo, > 30°. Pedidos: documentar/substituir o encosto de 2°; lei com a velocidade normal medida para apertar a tolerância. Merge após o push de bc3c856.
- **Lei alimentada pelo eixo das células (Netuno, fix/crater-law-along-the-axis cbaafeb; 22:12)**: a lei recebia o closing_speed ao longo da normal de CONTACTO (12–15° fora da rampa): 96,0 m³ a 10° para os 100,9 da mesma bola (−3 % de velocidade); `VoxelOwner::aligned` dá à cratera o eixo das células e a velocidade ao longo dele, e o frame usa o mesmo impacto alinhado (partículas e corte concordam); oráculo: volume da lei nas rampas = chão plano a 0,5 % (vermelho antes); destruído 106,3/103,0/99,6 m³ vs 100,9 (+5,4/+2,0/−1,3 %; o resíduo a 10° é a escada, dito como limite). Encosto de 2° documentado (ruído 0,1°; descontinuidade dita). Chão plano inalterado. Merge após o push de bc3c856.
- **3.4 fratura por tensão, variante B entregue (Saturno, feat/fracture-stress 3a0c0e0, 11 commits sobre b255d61; 22:20)**: `World3::with_stress`: um corpo até o critério disparar; cargas por junta por Newton–Euler sobre os impulsos do passo (peso, campos, contactos na peça tocada, junta ao mundo), exatas em árvores; ciclos pelo PLANO da junta com partilha por área (tração) e distância ao centro (flexão); balanço em torno do centro de massa no movimento relativo (ponto fixo do mundo dava 16 N/65 N·m espúrios em queda livre); critério σ_n, τ, σ_b, torção com secções exatas (`pieces::sections`, i128), Rankine ≥ strength, compressão pura = 0; ordem: ler no fim do passo, partir juntas no início do seguinte; separação pela maquinaria de split; a parte presa ao mundo fica, senão a maior; pó/overflow pela política. Oráculos: viga encastrada a 5e-4 com W e L exatos; 0,995/1,005 P*; coluna pendurada 0,99/1,01; coluna de pé nada; viga a rodar parte nas duas juntas do meio com momento/momento angular/energia a 1e-9 em 240 passos; quatro caminhos; identidade sem stress; bloco de 256 peças a 100 m/s parte 634 de 640 juntas em 251 corpos; custo 0,05 ms/passo (1,5 ms na primeira leitura). Esquema: fracture@mode=impact|stress, @strength (Pa), FRX13–FRX15, 294 asserções, corpus 428. Limite: bloco em repouso lê ~3,5e4 Pa pelo espalhamento da pressão de contacto. `sr_eval::voxels::stress` pronto para a ligação (W6 de Netuno). **fix/voxel-fixtures 26cfa98**: fragmentMinCells=1 nos válidos + teste corpus.rs. Decisões: o documento de stress é recusado pelo nome até W6 (as listas E23 já não existem em main); rebase dos dois branches em main após o push. Revisão independente a correr.
- **Saturno fc00100 (22:25)**: o 3,5e4 Pa do bloco em repouso é a tração de meio-vão de uma viga profunda apoiada nos quatro cantos (98,1 N·s por canto = m·g·dt; M = WL/8; 6M/bh² = 3,53e4 Pa), não ρgh (4,7e4, compressão da base); agora é um teste (a 3 %) sobre os 640 ciclos. Encontrou também a corrupção do ledger (3fce02d): descartar no rebase, main já tem a reparação de Netuno e a regra L. Plano de rebase aceite (E24 para mode=stress até W6; no_body flags viradas).
- **Verificação de `bc3c856` limpa (22:02–22:21), CLIPPY_EXIT=0**: sr-sim 499, sr-eval 593, sr-model 139, sr-3d 173, varreduras 38, sr-gpu 547/0, CLI 87/0, PROBE_MATCH, fmt, hygiene com a regra L. **Push a origin** b255d61..bc3c856 (ligação W1–W5, ponto de contacto, lint, ledger reparado + regra L, resumo). A seguir merge de fix/crater-law-along-the-axis (c139c36 + cbaafeb).
- **fix/crater-law-along-the-axis integrado = main 7c9ef4a (22:22)**: oráculo do chão inclinado + eixo pela superfície das células + lei com a velocidade ao longo do eixo. `vrun_7c9ef4a` lançada 22:23; push depois. Saturno rebaseia feat/fracture-stress e fix/voxel-fixtures em main; Mercurio rebaseia V.3.
- **Revisão de 3.4 (3a0c0e0), 22:30**: confirmados balanço, lado, sinais, atribuição, flexão, Rankine, secções, viga, ordem, esquema; a leitura em repouso (0,75 ρgh) é a flexão no corte do meio com a reação nas arestas: artefacto do solver, não "tensão do peso". **Defeitos**: (1) atrito lido de warmstart_tangent_world por ponto com FrictionModel::Simplified (um impulso por manifold, do último sub-passo, copiado em todos os pontos; torção ignorada): ~metade em regime permanente, arbitrário no impacto; erro vai para `left` ou para a âncora; (2) piscina de slots partilhada por dois corpos a partir no mesmo passo (índice fora → panic; pó errado com overflow_to_dust); (3) rebase: conflitos (refused.rs apagado, backends.rs, ledger) e o avaliador não tem caminho de stress (cai na fratura por impacto a t=0: E24 obrigatório). Qualidade: viga afirma 0,005 (doc diz 5e-4); 634/640/251 não afirmados; anel só desigualdade (4e5 Pa à mão); momento a 1e-8. Nits: c pelas esquinas, W_t sub-lê 13 %, validação da massa, cache FNV sem comparação. Enviado a Saturno; merge de 3.4 após o tip rebaseado e a revisão do rebase.
- **V.3 entregue (Mercurio, feat/voxel-surface 3188ac2, 14 commits sobre bc3c856; 22:35)**: mesher (faces por bitboard de bricks, fusão gulosa com dourados do mesher de referência em python), orçamento 1240 B/quad, SurfaceCache por (lineage, revision, classes) com update_known/update_steps (saltos 0→2→3→5 remalham a união dos bricks dos cortes em falta; passo ausente/revisão atrás/outro corpo = remalha total), junções em T (0 buracos em 20 imagens), W09 (sombra raster perde pixels com cellSize < 0,5: 6/5/9/23/51 px a 4/2/0,5/0,25/0,1; path tracer ≤ 5 até 0,05), **render_voxels.rs lê FrameNode::voxels**: cratera num chão de 240×40×240 com raster e path tracer: superfície pós-corte = remalha incremental (83 bricks) com o fingerprint da remalha completa; peças do próprio grid com pose3; renderer sem história desenha o mesmo quadro bit a bit; cobertura raster vs caminho 2 px de 8011 (0,02 %), ambos em borda. Identidade: nenhum .wgsl muda, sete imagens e hero c8a66483… iguais; sr-gpu completo e CLI 69 na NVIDIA. Não verificado: llvmpipe, uma câmara/luz, pontilhado de bias no chão plano (item), custo de CPU do corte em chão grande. Revisão independente a correr. Próximo de Mercurio: nota de 4.1 scatterBounces.
- **Verificação de `7c9ef4a` limpa (22:23–22:43), CLIPPY_EXIT=0**: sr-sim 499, sr-eval 594, sr-model 139, sr-3d 173, varreduras 38, sr-gpu 547/0, CLI 87/0, PROBE_MATCH, fmt, hygiene. **Push a origin** bc3c856..7c9ef4a (chão inclinado: eixo pela superfície das células, lei pela velocidade ao longo do eixo).
- **Revisão de V.3 (3188ac2), 22:50**: confirmados exposição, vidro, fusão, 65 535 como divisão, dourados reproduzidos em python, aritmética, update_steps, transformações, tempo de vida, W09, identidade. **Defeitos** (merge espera): materiais animados congelam (assinatura só com ids; material por frame e see-through na assinatura); orçamento não para no primeiro plano no caminho de produção; pico real ~2,1 KB/quad (Quad 24 B, cópias e malhas antigas vivas); orçamento por corpo e não por objeto como o XSD diz; W09 só por object3D@cellSize (não vê asset/srvol/escala; mede tamanho no mundo; conselho silencia sem corrigir); node_hash sem termo de voxels (corte não muda o hash de um grupo isolado em cache); update() pode devolver parcial obsoleto após compactação (latente). Qualidade: remeshed < 390 não pode falhar; heurística ½ sem teste; 4 medidas de W09 não afirmadas; cobertura a 1 % vs 2 px; incremental vs total só consistência. Nits: entradas não removidas; varrimento de todas as células por frame; erro de device silencioso; duplicação. Enviado a Mercurio.
- **3.4 rebaseado com as correções (Saturno, fix/voxel-fixtures 92336a5 + feat/fracture-stress 0fa8311 sobre 7c9ef4a; 23:00)**: fixtures (fragmentMinCells=1; asset novo block.vox 16×4×16 por make_corpus_block.py; o documento de crater do corpus falhava no ÚLTIMO frame ("no revision at this frame" + "crater no larger than the projectile"): o teste de Netuno só via o primeiro e o meio; corpus.rs avalia até ao último frame e falha com qualquer falha/problema); E24 para mode=stress em pending.rs (corpus.rs aceita só esse e exige exatamente um); atrito lido uma vez por manifold no meio dos pontos carregados e escalado ao total do passo (n_sub com junta ao mundo; senão pelo resto do balanço), ponto de contacto do próprio corpo, aresta entre peças procurada para dentro, sinal corrigido; rotação do MEIO do passo para o referencial da carga (era a do fim: 2,5 % de corte → 30 % de flexão em corpo esbelto; viga a rodar com tração centrípeta a 2e-3); slots reclamados em turno com prepare cumulativo (dois corpos a partir no mesmo passo com 3 slots: erro nomeado ou pó); viga 1e-3 (medido 4,5–5,1e-4), bloco 633/640 e 250 corpos, anel 4,19e5/2,04e4 Pa nos cantos, momento 1e-9, densidade única validada, cache do plano por conteúdo; docs. Abertos: leitor de torção; escala do atrito com junta ao mundo e atrito. Revisão curta a correr.
- **Regra de todos os frames (Netuno, test/every-frame-and-one-failure 57c8de3; 23:08)**: assert_every_frame_is_clean aplicado à cratera, burst, 3 fraturas e rampa de 20° (todos limpos; o achado era do fixture do corpus); correção: quando o mundo falha, cada objeto de células acrescentava uma segunda falha repetida ("no revision at this frame"): agora só a causa ("loose parts … maxFragments"); nota no ledger. A integrar em lote com fixtures + stress.
- **Revisão das correções de 3.4 (23:12)**: fixtures limpos (block.vox byte-idêntico; a bola antiga removia o asset inteiro; último frame; 12 docs; um E24); stress: rotação do meio exata, 138 240 Pa, anel 4,186e5/2,035e4 recomputado, slots determinísticos, instalação tudo-ou-nada, cache por conteúdo; **três defeitos ficam**: (1) com junta ao mundo a escala do atrito é n_sub × último sub-passo (o Rapier guarda o normal total e o normal do último: escalar por Σimpulse/Σwarmstart_impulse; num impacto hoje sub/sobre-lê até 0 ou n×; sem teste com junta e atrito); (2) o ajuste do corpo livre é circular (o rácio elimina-o); (3) braços dos contactos em pontos do fim rodados pela rotação do meio (erro (θ/2)·|braço|·|J|; formar os braços no referencial do corpo); (4) include escapa ao E24 (program.rs:1918). Qualidade: corte não afirmado; mesmo passo não afirmado; sum == 4; reservas antes do laço. **Merges = main 149badb**: fix/voxel-fixtures 92336a5 + test/every-frame-and-one-failure 57c8de3 (corpus 423, oráculo ok). `vrun_149badb` lançada 23:13; push depois. Stress espera o tip.
- **Verificação de `149badb` limpa (23:13–23:22), CLIPPY_EXIT=0**: sr-sim 499, sr-eval 599, sr-model 139, sr-3d 173, varreduras 38, sr-gpu 547/0, CLI 87/0, PROBE_MATCH, fmt, hygiene. **Push a origin 2026-10-08 09:09** 7c9ef4a..149badb (fixtures do corpus limpos até ao último frame, E24, regra de todos os frames). Pendentes: stress (3 defeitos), V.3 (7 defeitos), W6.
- **V.3 corrigido (Mercurio, 7ce8174 sobre 3188ac2; 2026-10-08 09:20)**: material resolvido por frame com see-through na assinatura (Renderer gasto = fresco; transmissão a cruzar 0 muda 20→22 triângulos); orçamento verificado a cada plano (tabuleiro recusado no 3.º plano); pico contado 888 B/quad com malhas antigas libertadas antes (151 146 quads a 128 MiB); orçamento único do objeto; W09 pelo tamanho na cena (objeto, senão asset, × menor escala; 14 bandas afirmadas; renderizador anota qualquer voxels < 0,5); termo de voxels no node_hash só quando Some (corte redesenha o grupo isolado); marca de compactação + guarda; qualidade (planos == união dos bricks; 83 afirmados; área = faces vazias contadas; cobertura ≤ 0,1 %); nits. Rebase em 149badb e verificação completa a caminho (~1 h). 4.1: o estimador numpy bate na placa isotrópica mas diverge do fotão analógico a g≠0 (0,392 vs 0,327): causa antes da nota.
- **Oráculos da cratera oblíqua e em chão curvo (Netuno, nota 09:30) aprovados**: oblíquo (rampa de 20°, mesma velocidade normal 86,6 m/s, tangencial 86,6·tan φ a 30° e 60°, na linha de queda e atravessada): lei = chão plano a 0,5 %, eixo < 3° da normal, centro < 0,5 m do ponto da geometria (esperado a falhar a 60°: o ponto entra pelo passo ao longo da velocidade e surface_along projeta ao longo do eixo; primeira correção: recuar ao longo da velocidade e só depois projetar), destruído em banda de 6,5 % e dentro do alcance, momento tangencial dos lançados = 0 (limite declarado: a lei não tem enviesamento a jusante), todos os frames; curvo (campo de alturas 30×30×12: colina a 20 e 10 m de curvatura no topo e a 5 m, bacia, crista, beirada): destruído = força bruta exata {r < crista+rim ∧ a ≥ S(r) ∧ a ≤ teto}, eixo vs normal analítica medido a 40/20/10/5 m (raio do estimador decidido pelo erro), lei = plano a 0,5 %, rim completo ou falha registada como resultado (esperada na bacia), conservação, todos os frames. Fora: > 30°, dono móvel. Branch test/crater-oblique-and-curved; entregas por grupo; W6 interrompe quando o stress entrar.
- **3.4, estado (Saturno, 09:40; ETA 1,5 h)**: braços no referencial do corpo no meio do passo (viga a rodar com contactos nas pontas: vermelho sem a mudança); atrito pelo rácio Σimpulse/Σwarmstart_impulse por manifold, sem projeção circular, com e sem junta ao mundo, mais tampão de Coulomb (μ·Σnormal): exato em deslizamento estável (m·Δv a 2e-3); na viga soldada com bloco a aterrar a deslizar 4–6 % acima e 23 % abaixo no passo em que para; passos SEM vetor de atrito (aterragem, ressalto: o Rapier 0.36 não guarda sub-passos anteriores) leem 0 — limite dos dados, a declarar com números e consequência para o critério; rácio instável (4101) tratado pelo tampão; barra a rodar deitada: resíduo = I·Δω afirmado (twist não lido). Falta: include para E24, itens de qualidade, docs, rebase, verificação.
- **Ordem do usuário (2026-10-08 09:17)**: "merge into main and push when ready": os branches entram em main e vão a push assim que a revisão e a verificação completa de cada um estiverem limpas (stress 3.4, V.3, oráculos da cratera), sem esperar nova ordem.
- **3.4 entregue, revisão 2 tratada (Saturno, feat/fracture-stress 110e66a sobre 149badb; 09:50)**: 40a6c68 atrito pelo rácio Σimpulse/Σwarmstart por manifold com teto de Coulomb e sem ajuste ao balanço (bar-slides: rácio 8,00, m·Δv a 2e-3; welded-block; spinning-bar: resíduo = I·Δω); 4f9128c braços no referencial do corpo pela rotação do meio (atan2); 97d493f E24 também em includes; f0adf4a qualidade (corte à parte; duas metades no mesmo passo com ids exatos; reservas no laço; regra do pó por índice; bloco com limites ≥ 600 juntas e 200..=256 corpos, 633/250 como nota; nits); 110e66a docs com os limites de dados do atrito em números (0 na aterragem e ressalto; 4–6 %/23 %). Verificação local limpa (sr-3d 178, sr-sim 540, sr-model 140, sr-eval 606, corpus 428, clippy, fmt, hygiene). Revisão curta a correr; merge, verificação e push a seguir.
- **V.3 corrigido e rebaseado (Mercurio, feat/voxel-surface 3d67289, 26 commits sobre 149badb; 09:55)**: os 7 defeitos e nits (ver 09:20) + cast do clippy 1.98 + corpus regenerado: 6 documentos válidos de voxels com cellSize 0,25 passam a válidos com aviso W09 e 20 inválidos ganham W09 (decisão: ficam a 0,25 com o aviso; são documentos de física). Verificação NVIDIA limpa (CPU, sr-gpu todos os grupos, CLI 69; identidade das sete e hero c8a66483… a 22,7 s). 4.1: a divergência a g≠0 era do oráculo (fotão binado só por cos); com o fotão resolvido em azimute o estimador bate (g=0,6: 0,389 vs 0,392; g=−0,5: 0,457/0,281 vs 0,453/0,281); nota a seguir; padrão de identidade = scatterBounces=1. Revisão curta a correr; merge com o stress, verificação, push.
- **Oráculos da cratera, grupos 1–2 (Netuno, test/crater-oblique-and-curved 8c76d05; 10:05)**: oblíquo a 20° (φ = 30°/60°, linha de queda e atravessado): achado imprevisto: a 30° na linha de queda cortou +13,7 % porque o plano do kernel era posto onde o eixo encontra a ESCADA (até meio degrau acima/abaixo do plano médio = ±12 % do volume); correção: plano médio ajustado pela fração de células cheias no raio da crista até meio raio abaixo (F(u) = ½ + (3u − u³)/4, bissecção), só em superfície inclinada (chão plano inalterado), com guardas (bola < 200 células; chão mais fino do que a fatia: um esboço sem guardas partiu o documento do corpus); rampas 10/20/30° passam a −1,0/−1,0/−1,3 % e os 4 oblíquos a −0,8…−1,3 % (bandas 2,5 %); lei = plano a 0,5 %; eixo a 0,03–0,09°; momento tangencial ≈ 0 (limite declarado); **centro da cratera a 0,25/0,77/0,77/1,04 m do ponto tocado** (média ponderada por impulso + percurso tangencial + base da bola): limite registado com margem 1,2 m (um sexto do raio da crista). Diagonal (15°+15°, normal a 20,7° do eixo): passou à primeira (−2,0 %; eixo a 0,06°). Corrida de diretório temporário corrigida. Revisão curta a correr; merge em lote com stress e V.3.
- **Revisão 3 de 3.4 (110e66a), 10:15**: confirmados rácio com fallback e manifold saltado a 0, ajuste circular removido, braços (vermelho 16,6 N·m·s), E24 em includes, slots após saltos, duas metades, docs, compatibilidade. **Três defeitos**: (1) teto de Coulomb com max(μ1, μ2) quando os colisores normais combinam por Average (μ real 0,5 vs teto 0,8); (2) teto engatado = μ·total na direção do último sub-passo mesmo numa aterragem a direito sem deslizar: corte falso no passo da aterragem; (3) stress_install_for antes dos saltos: um corpo saltado pode abortar todos os cortes da chamada. Qualidade: welded-block não distingue o rácio da escala fixa (normal estável) e salta zeros; bodies ≤ 256 não pode falhar; dead code. Nits: spin restante no braço do fim; friction_scale sem teto; docs do E24 em includes; frase do pó invertida. Stress espera mais um tip; V.3 e oráculos seguem.
- **4.1 scatterBounces — nota de desenho (Mercurio, 10:20) aprovada**: nota e modelo numpy em /home/pals/renders/cinematic-impact/phase3/volume-scatter/; hoje a dispersão é só de 1.ª ordem (forno branco dá 0,30, não 1); scatterBounces = máximo de colisões por caminho, padrão 1 = hoje (shader byte a byte igual), 0 = sem dispersão, máx 32; estimador: reservatório ponderado sobre os passos do march + passeio HG + NEE por vértice + roleta russa, termina na 1.ª superfície; distância livre por transformada inversa sobre a profundidade ótica marchada (colisão nula como alternativa); validado em numpy: placa exata 0,5636 vs 0,5652, albedo 0,8 0,2900 vs 0,2904, 1.ª ordem fechada, g=0,6/−0,5 vs fotão ≤ 1 %, forno branco 0,995 a n=40; variância ~0,9 da média por caminho (~22 % por pixel a 16 amostras); oráculos e plano em 6 commits; custo só após sonda na pluma. Decisões: 0 permitido; commits 1–2 já; 3–6 vermelho primeiro; variância medida e estratificação do reservatório como primeiro remédio. Branch feat/scatter-bounces.
- **Revisão dos grupos 1–2 (8c76d05), 10:30**: F(u) e a normalização confirmadas (região amostrada exata; 45° dá 0,407 por truncatura), bissecção, chão plano exato com pinos intactos, bandas, lei. **Defeitos**: a corrida do diretório temporário volta no grupo 2 (o teste inclinado e o normal partilham "slope-along-0-0-false"); rampas < 2° não recebem o plano médio (snap ⇒ flat ⇒ terraço do ponto de contacto: ±16 % de volume a 1–1,5°), sem teste nem menção. Qualidade: guardas sem testes unitários (remover qualquer uma não falha nada; borda lateral passa a guarda de espessura); comentário "duas células" vs 1,2 m; os quatro desvios impressos, não fixados; a decomposição do desvio não bate com os números (0,625/0,21 m por passo; 0,4 m não medido; a escada do teste fica 0,117 m acima do plano analítico); momento tangencial ≈ 0 por simetria azimutal (não é física); `along ≤ radius + 0,5` não pode falhar. Enviado a Netuno; merge após o tip.
- **Revisão 2 de V.3 (3d67289), 10:35**: os 7 defeitos confirmados corrigidos (material por frame com teste gasto = fresco; orçamento por plano; 888 B/quad; orçamento por objeto; W09 com 14 bandas; node_hash; compactação; corpus com 6 válidos em warnings). **Dois defeitos novos**: peça recusada pelo orçamento é expandida e subida de novo a cada frame (update_steps devolve Ok cedo sem verificar orçamento); a nota de células pequenas entra em stats.unsupported: `--strict` sai com 1 para qualquer cena de voxels < 0,5 (incluindo o chão da cratera a 0,25 do ficheiro), diz "not rendered yet" (falso) e o sidecar incremental nunca regista esses frames. Qualidade: 888 não cobre o path tracer; teste de hash sem voxels não pode falhar; `<= 3`; planos por contagem; comentário; arredondamento; "+3"; mensagens; compact() sem chamador. Enviado a Mercurio; merge após o tip. Estado: os três branches (stress, V.3, oráculos) aguardam um tip cada.
- **3.4 integrado = main a54089e (10:00)**: Saturno a7119c3 sobre 110e66a: d0b0aba (guardas antes do install; corpo saltado não pede slots nem corta só juntas), cdd7ad4 (teto pelo coeficiente combinado pela regra de cada colisor: 0,8/0,2 → 0,5; vermelho só no unitário, dito), 482aea5 (rácio acima do teto: teto se o manifold desliza (> 1 mm/s), senão não lido; a aterragem na ponta da viga soldada DESLIZA (1,46 cm/s), logo o teto é Coulomb aí; unitário com ruído × 4101 → None), 860b970 (welded-block afirma os passos sem atrito [16, 27] e os cinco passos com rácio ≠ n_sub a 10 %; bloco 256 − corpos ≤ juntas intactas; dead code), a7119c3 (docs; "23 % abaixo" era no passo 50 com normal a mudar 6 %, não ao parar). Lido por mim (friction_of_step, combined_friction, guardas). Corpus 428. `vrun_a54089e` lançada 10:00; push depois. W6 de Netuno pode começar sobre main local.
- **V.3, tip 3 (Mercurio, 6a67c71; 10:10)**: f0546db (peça recusada não é expandida nem subida a cada frame: quad_count × 888 contra o que resta antes de expandir; budget_error partilhado por cache, mesh_quads_within e renderizador; uploads = 0 no 2.º frame), edde9d0 (regra W09 lê xs:double: "+0,25" avisa, 0,49999 avisa, 0,5 não; corpus alinhado), 6a67c71 (nota de células pequenas fora do renderizador: --strict passa e --changed-only regista, teste CLI vermelho antes; planos como conjuntos derivados da diferença das grelhas; planes_meshed == 3; SREP: 888 só do raster/cache; docs). Teste do hash fica estrutural (nit). Verificação NVIDIA limpa (CLI 70, scene_3d 157). Lido por mim (verificação do orçamento antes de expandir). Merge após o push de a54089e.
- **Oráculos da cratera, tip de revisão (Netuno, cc48191; 10:15)**: 78d8e53 Dir::new recusa nome repetido no processo (vermelho: o inclinado); cc48191 rampas < 2°: vermelho +15,5 % a 1°; plano = altura média do chão no disco do alcance inteiro ponderada pela extensão ao longo do eixo, substitui o ponto exato só acima de 1/6 célula (−1,0 % a 1°, −1,1 % a 1,5°; plano exato intacto); 7 testes de guardas (cada remoção falha); penhasco lateral recusado (fundo 95 % chão) com fallback ao ponto dos degraus; desvios fixados por caso a 0,05 m (0,343 normal; 0,252/0,768/0,775/1,035) como medidos, sem decomposição; passo físico 1/240 e escala dita (1,25 m a 1/120); momento tangencial rotulado como simetria; alcance e teto exatos; margem diagonal dita. Grupo 3 (colina): estimador do eixo a meia crista (inteira abaixo de 4 células) aprovado (0,2° vs 12,1°/18,7° a 10/5 m de curvatura); todas as colinas falham em heap_rim ("room for 0 of 1059": chão que cai abaixo do plano não segura o rim): resultado registado; correção após W6. Regra lembrada: nada em git stash (WIP commit). Merge com V.3 após o push do stress.
- **Verificação de `a54089e` limpa (10:00–10:22)**: sr-sim 546, sr-eval 606, sr-model 140, sr-3d 178, varreduras 38, sr-gpu 547/0, CLI 87/0, PROBE_MATCH, CLIPPY_EXIT=0, fmt, hygiene. **Push recusado**: o usuário fez push de três commits (94e367c rasterização CPU/ordenação de splats/reuso de geometria rígida; 91e8045 asserção de determinismo das ejeta + validação do blast simplificada; e834a55 formatação). **Merges (10:25–10:30)**: feat/voxel-surface 6a67c71 (conflito em tests/corpus/manifest.json: regenerado por build_corpus) = 96b7bfe; test/crater-oblique-and-curved cc48191 = 6088b87; corpus regenerado após os dois merges (W09 nos documentos de stress a 0,25: 5 ficheiros) em commit próprio; merge de origin/main. Verificação lançada; push depois.
- **W6 entregue (Netuno, feat/voxel-stress-wiring 4202829 sobre a54089e; 10:35)**: o avaliador lê fracture@mode=stress; stress_of chama voxels::stress com a partição própria (voronoi/planos/labels); with_stress depois das divisões; corpo com VoxelOwner e slots (frames pelo caminho existente); falhas nomeadas (não é corpo de células; não dinâmico; crater no mesmo objeto); E24 retirado (pending.rs, program.rs, filtro de include, codes.rs, stress_refused.rs, exceção em corpus.rs); oráculo scene_stress.rs: viga de 5 células (0,4 m, 2400 kg/m³) soldada a uma parede estática com planos em cada célula, strength 8e5 Pa, caixa na ponta; P* por bissecção (as outras juntas pedem ≥ 1,3 P*); 0,995 P* nada em 2 s e viga a 5 mm; 1,005 P* só a raiz, as quatro células saem como UMA peça que cai com g a 3 %; células do corpo + peça = fonte; fragmentMinCells=5 faz pó; strength 1e18 bit a bit igual ao documento sem fratura; todos os frames limpos. Limites: pó sem partículas; piscina de slots por objeto; só o peso na ponta tem oráculo; cache recusada. Grupo 3 em WIP commit (wip/crater-hill ec1cd13), não em stash. Revisão independente a correr; merge após o push de b302f30.
- **3.5 canal de vapor — nota de Saturno (10:45) aprovada**: terceiro escalar advectado `vapor` (kg/m³) no pyro, com `condensation` (1/s) e massa condensada registada; massa de um evento = vaporFraction × E / L com L = c_w·(373,15 − T_água) + L_v (4186 J/kg/K; 2,256e6 J/kg), limitada pela água disponível na pegada; `pyroVapor` (filho de pyro: tempo, posição ou crater@source, energia, fraction, duração, perfil gaussiano, ocean=IDREF) injeta por passo em células livres, temperatura 373 K pelo gancho `heated`, expansão a 1 atm (ρ_v = 0,588 kg/m³) como divergência-alvo (modelo do êmbolo); exige boundary=open; segue o inject_piston e a janela Follow. Oceano: sumidouro de massa por célula no passo canónico, registo `evaporated` no Frame, lido pelo pyro (conservação por construção; ordem oceano → pyro). Render: sexta grelha `vapor` no SRVOL só quando existe; duas VolumeDraw com Medium/albedo por canal (Mercurio). Oráculos 1–7 + 8 (fecho com condensação). Custo +8 B/célula ×3 só quando declarado. Limites v1: sem calor latente devolvido, vapor sem peso próprio, evento único, sem ebulição por corpo quente. Decisões: oceano manda com registo; duas VolumeDraw; nomes aceites; ordem A (pyro) → B (oceano) → C (esquema/eval) → D (render); branch feat/pyro-vapor após o push; leitor de torção primeiro.
- **Revisão de W6 (4202829), 10:50**: sem defeito no avaliador (despacho só com mode=stress; partições; ordem; frames; cache; E24 retirado; bissecção; identidade de poses em 48 frames). Itens para um tip pequeno antes do merge: frase falsa no ledger sobre o documento do corpus ("cai sem carga": a bola de 500 kg a 120 m/s atinge a laje a ~0,07 s e a rotura é provável: afirmar o que acontece); a rotura por tensão dá revision = passo+1 sem edit step (renderizador faz remalha total): uma edição por rotura com bricks mudados, afirmada; recusas inalcançáveis em sim3d.rs:719/721/725; partição duplicada; verificação de g num só tripleto sem every-frame; os 5 mm não distinguem rotura; ledger: ≥ 1,3 P* é aritmética; oráculo = modelo do motor; custo dos slots não medido; digest sem tensões (seguro só pela recusa da cache).
- **Leitor de torção do manifold (Saturno, feat/manifold-twist ec0a3c5 sobre b302f30; 10:55)**: warmstart_twist_impulse (manifold com > 1 ponto) lido como o atrito (último sub-passo × rácio das normais; teto μ·Σ|normal|·distância ao meio do patch; acima do teto é o teto se os corpos rodam um sobre o outro > 1 mrad/s, senão não lido); sinal medido no Rapier (binário sobre o primeiro corpo ao longo de −normal); entra no balanço como Couple{moment, piece}; testes: barra a girar a 5 rad/s com torção = I·Δω a 1e-8 e resíduo 1e-9 nas duas ordens do par; unitário da viga com couple na ponta; StressBalance.twist diagnóstico; docs. Verificação local limpa (sr-3d 212, sr-sim 548, sr-model 145, sr-eval 616). Lido por mim. Merge com o W6 após o push de b302f30. Perguntado se o item "junta ao mundo E atrito (n_sub)" ainda existe após 40a6c68.
- **Item "junta ao mundo E atrito (n_sub)" fechado (Saturno, 11:00)**: coberto pelo rácio desde 40a6c68; o único `substeps` restante é o fallback sem normal no último sub-passo (vetor zero, saltado) e o diagnóstico; evidência: welded-block (cinco passos com rácio 1/4101/11/15/19 a 10 %; passos 16 e 27 a zero afirmados). Sem commit (só o meu registo o listava). Saturno espera o push para abrir feat/pyro-vapor (passo A).
- **PARAGEM por ordem do usuário (2026-10-08 10:51)**: "os trabalhos estão rodando de maneira indefinida; não estou tendo visibilidade; pare todas as atividades e me apresente um plano de ação". Verificação de b302f30 abortada (sr-3d 212 passado; faltavam GPU, sonda e lints); os três agentes instruídos a parar (WIP commit, sem builds, uma linha de estado). Estado: origin/main = e834a55 (do usuário); main local = b302f30 (66 commits à frente, não verificado até ao fim, não pushed). Branches entregues e não integrados: feat/voxel-stress-wiring 4202829 (W6, revisto, tip pequeno pendente), feat/manifold-twist ec0a3c5 (lido). Em curso: wip/crater-hill, feat/scatter-bounces (commit 2). Plano de ação apresentado ao usuário; retoma só com aprovação.
- **Ordem do usuário (10:55): concluir o passo 1 = push para main.** Decisão (a): sem o W6 (WIP com erro de compilação; entra no push seguinte). feat/manifold-twist integrado = main f64664e; `vrun_f64664e` lançada 10:56 (verificação completa única); push assim que limpa. Agentes continuam parados.
- **Verificação de `f64664e` limpa (10:56–11:08)**: sr-sim 548, sr-eval 616, sr-model 145, sr-3d 212, varreduras 38, sr-gpu 569/0, CLI 88/0, PROBE_MATCH, CLIPPY_EXIT=0, fmt, hygiene. **Push a origin** e834a55..f64664e (67 commits: 3.4 fratura por tensão, V.3 superfície de voxels, oráculos da cratera em rampas/oblíquos, torção do manifold, corpus regenerado, commits do usuário fundidos). Passo 1 concluído. Agentes parados à espera da aprovação do passo 2.
- **Limpeza por ordem do usuário (12:05)**: apagados 66 branches locais já integrados em origin/main (64 não em checkout + fix/ci-format após remover o worktree /tmp/rs-scene-render-ci + o duplicado feat/voxel-asset-on-0c9c07e); agentes instruídos a destacar os worktrees base (saturno-pyro, netuno-ocean) para origin/main e a remover mercurio-voxel (branches feat/manifold-twist, perf/crater-collider, feat/voxel-surface apagam-se depois). Ficam: main (realism), cinematic-impact-checkpoint (checkout principal do usuário), feat/scatter-bounces (mercurio-scatter), feat/voxel-stress-wiring (wt-density, W6 WIP), test/crater-curved e wip/crater-hill (colinas), parked/ocean-sponge (estacionado). Remotos: nenhum branch remoto integrado a apagar (os 20 restantes são gap/* e outros do usuário, não integrados).
- **Limpeza completa (12:15, ordem repetida)**: removidos os worktrees saturno-pyro e netuno-ocean (destacados em origin/main, limpos); o checkout principal /home/pals/src/rs-scene-render passou a HEAD destacado em origin/main b08cb2b e o branch cinematic-impact-checkpoint (integrado) foi apagado. Restam: main (worktree realism), feat/scatter-bounces (mercurio-scatter), feat/voxel-stress-wiring (wt-density), test/crater-curved, wip/crater-hill, parked/ocean-sponge. Disco: 115 GB livres. Agentes avisados (worktree novo a partir de origin/main ao retomar).
- **Para quando este branch chegar à produção:** `tools/production_gate.py` valida cenas
  de filme com `--deny-warnings`; com o W02, uma cena com fonte de fumaça a menos de 12
  células de uma face aberta passa a ser barrada pelo portão. É o efeito pretendido, mas
  muda o veredito do portão para cenas nessa condição.
- Segundo erro de processo meu do mesmo tipo: relatei a grade como integrada antes de
  conferir o merge, que tinha falhado por causa dos hashes antigos. O script de
  verificação passou a abortar se o topo da integração não for o commit esperado.
- Falhas do adaptador de software classificadas (Mercurio; cada teste 3 vezes, isolado, na
  base `60a458c`, em `f21d465` e em `89249df`, com carga ~12): nenhuma regressão
  demonstrada. Determinísticas já na base: `morphology wide_morphology…`,
  `pathtrace_instances`, `text caption_source_newlines…`, `volume advected_cache…`.
  Instáveis em todos os commits: `raster solid_fill…`, `cache_regressions caption_cache…`,
  três de `composite`, `maps routes…`, `text per_character_blur…`. **A conferir com a
  máquina quieta no fechamento:** `effect_costs layers_whose_content_changes` (3/3 na
  base, 1/3 na integração, 0/3 no topo) e `composite polygons_and_stars…`; dez execuções
  por commit e bisseção se a diferença persistir. Todas passam na NVIDIA.
- Erro de processo meu: um merge parou em conflito no SREP sem eu perceber e a
  verificação rodou sobre a árvore em merge inacabado. Corrigido; a verificação passou a
  contar arquivos não resolvidos e marcadores de conflito.
- Observação para as fases seguintes: em escala de quilômetros, tudo que depende da
  gravidade é lento em tempo real (uma onda leva minutos). Um plano cinematográfico de
  um impacto grande vai precisar de escala de tempo uniforme na simulação, que hoje os
  membros de um grupo acoplado não aceitam.

**2.0 Agendador de co-simulação (pré-requisito de tudo que é mão dupla)**

Novo módulo em `crates/sr-eval` (por exemplo `cosim.rs`):

- Solvers que se referenciam formam um **mundo acoplado**; cenas sem ligações continuam
  no caminho atual, sem mudança.
- Passo macro comum; cada solver subdivide com o seu `dt` (exigir passos comensuráveis,
  com regra de validação nova).
- Ordem fixa de troca: rígido → oceano → líquido → fumaça → partículas. As forças e fontes
  calculadas no passo k são aplicadas no passo k+1 (acoplamento explícito, atraso de um
  passo, determinístico).
- Checkpoint conjunto: todos os estados no mesmo passo, sob um orçamento único; replay
  reverso repete o mundo inteiro.
- Cache de física ganha versão nova (hoje `SRPHYS03`) para registrar trocas.

**2.1 Oceano ← fundo e corpos**

- `advance` passa a receber o fundo do passo, não um vetor fixo. O fundo é amostrado da
  malha deformada pela cratera (a amostragem de topo de malha já existe em
  `crates/sr-eval/src/ocean/bathymetry.rs`).
- Fundo que sobe ou desce desloca a coluna de água inteira: a profundidade é mantida, a
  superfície acompanha, e a gravidade irradia a onda. É a forma padrão de iniciar tsunamis
  e conserva água exatamente.
- Corpos fechados dentro da coluna contam como ocupação (elevação efetiva do fundo) e
  transferem quantidade de movimento horizontal.
- Esquema: `ocean@colliders` (IDREFS), coerente com `pyro` e `particles3D`.

**2.2 Corpos ← água**

- Empuxo pelo volume submerso sob a superfície local do oceano e arrasto proporcional à
  velocidade relativa, aplicados como forças externas no passo rígido.
- Esquema: `ocean@density`.

**2.3 Contato como fonte**

- Registrar, por passo, um log determinístico de contatos do Rapier: instante, ponto,
  normal, impulso, velocidade relativa, corpos envolvidos (`crates/sr-sim/src/physics3d.rs`).
- Mecanismo genérico para consumidores: `on="contact"` + `body` (IDREF) + fração de energia.

| Consumidor | Hoje | Passa a ser |
|---|---|---|
| `crater` | `radius`, `depth`, `start`, `end` autorais | `source="corpo"`: raio e profundidade por lei de escala de cratera (grupos π, Holsapple/Schmidt–Housen) a partir da energia e do ângulo; centro no ponto de contato; constantes de material como atributos |
| `particles3D` | `burst time` + `speed`/`spread` autorais | Emissão no contato: massa igual ao volume escavado; velocidade e ângulo por posição radial (modelo Z de Maxwell), com viés na direção do impacto oblíquo |
| `pyroSource`/`pyroImpulse` | `time`, `temperature`, `expansion` autorais | Fonte no contato: calor e densidade a partir da fração de energia dissipada |
| `fracture` | Ativação por tempo | Ativação quando o impulso de contato passa de um limiar |

**2.4 Partículas ↔ resto**

- Fumaça → partículas: arrasto em direção à velocidade do gás, via `Driver::acceleration`
  usando `State::velocity_at` (ambos já existem).
- Partículas → oceano: partícula que cruza a superfície gera impulso local e espuma, e
  afunda ou é removida.
- Partículas → fumaça: ejetos quentes deixam rastro de densidade.
- Fumaça → corpos: guardar o campo de pressão da projeção e integrar sobre os corpos
  voxelizados para obter força.

Critério de aceite:

- Cena de aceitação sem nenhum `time` em efeitos: só corpo com velocidade inicial, oceano,
  fundo e ligações. `waterImpulse`, `burst time` e `pyroImpulse time` ausentes.
- Testes de monotonicidade: dobrar a velocidade aumenta raio da cratera, altura da onda,
  alcance dos ejetos e altura da pluma; mudar o ângulo desloca a distribuição de ejetos
  para o lado oposto à chegada.
- Conservação: volume de água constante a menos de arredondamento com fundo móvel; massa
  de ejetos igual ao volume escavado dentro de tolerância declarada.
- Lago em repouso continua em repouso com fundo estático (propriedade de equilíbrio).
- Replay reverso do mundo acoplado idêntico bit a bit, inclusive após descarte de checkpoints.
- Cenas existentes inalteradas (suíte atual sem mudança de valores).

### Fase 3 — Física que falta (4–8 semanas) → 78

- Item de desempenho vindo da Fase 2 (medição de Netuno, 2026-10-05): um passo do mundo
  rígido com a rocha em contato com a malha da cratera que deforma custa 24–33 ms,
  contra 0,02 ms livre e 3 ms em repouso; na cena do mar isso é 10% do quadro. Qualquer
  fase que ponha vários corpos sobre terreno deformável precisa de um colisor da cratera
  mais barato (atualizar só as facetas tocadas, ou campo de altura analítico no contato).

É a fase mais longa e a de maior risco técnico.

**3.1 Líquido 3D local**

- Novo solver FLIP/APIC em grade MAC, em um domínio local ao redor do impacto. Extrair de
  `pyro.rs` um módulo MAC comum (projeção, obstáculos, bordas) e reutilizar.
- Superfície livre com pressão zero na interface (ghost fluid).
- Acoplamento com o oceano raso em uma faixa de sobreposição na borda lateral: o raso
  fornece altura e velocidade de entrada; o líquido devolve fluxo.
- Superfície: partículas → campo de distância → malha; entregue pela infraestrutura de
  malha de topologia variável e de bake que já existe.
- Espuma, respingo e bolhas gerados por critérios do líquido (ar aprisionado, curvatura,
  velocidade), substituindo o traçador atual dentro do domínio.
- Esquema: filho novo do `ocean` que declara a região 3D e seus orçamentos.

**3.2 Onda de choque**

- Etapa A (recomendada primeiro): fonte analítica de explosão pontual (Sedov–Taylor)
  alimentando expansão da fumaça, forçamento de pressão sobre a superfície do oceano e
  impulso nos corpos. Barata, determinística e validável contra a solução de similaridade.
- Etapa B (só se A não bastar): solver compressível de volume finito na grade da fumaça
  para os primeiros instantes, entregando o estado ao solver incompressível.

**3.3 Escavação e ejetos**

- Crescimento da cratera por campo de escavação (cratera transitória, colapso, borda
  final) em vez de interpolação entre `start` e `end`.
- Deposição: ejetos que pousam elevam o terreno (manto de ejetos).

**3.4 Fratura por tensão**

- Manter o pré-corte geométrico atual; a ativação passa a propagar o impulso por um grafo
  de ligações entre peças, que se rompem quando a tensão excede a resistência do material.

**3.5 Vapor**

- Corpo ou gás quente em contato com água converte massa de água em densidade de vapor por
  balanço de energia. Exige um segundo canal de densidade (poeira e vapor com albedos
  diferentes) na fumaça e no formato SRVOL.

**3.6 Detritos granulares (opcional)**

- Contato entre partículas para ejetos densos e deslizamentos. Prioridade baixa; decidir
  depois de ver o resultado de 3.3.

Critério de aceite: casos canônicos com referência publicada (seção 6), cada um como teste
automatizado com tolerância declarada, mais os testes de determinismo e de orçamento de
cada solver novo.

### Fase V — Voxels como primitivos de modelação (frente nova, 2026-10-07, pedido do usuário)

Hoje o motor trata voxels só como campos (fumaça, volumes `srvol`/OpenVDB, meios). Esta frente
acrescenta voxels como **geometria**: objetos feitos de células ocupadas com cor e material,
que se renderizam, colidem, fraturam e se destroem célula a célula. Corre em paralelo com a
Fase 3 e usa o que já existe: bricks esparsos de 8³, o voxelizador de colisores, a cratera
como deformação, o traçador e o raster de malhas.

Escopo da primeira versão (V1):
- **V.1 Ativo de voxels:** `<voxelAsset id src format="vox|srvol"/>` (MagicaVoxel `.vox`
  com paleta; `srvol` com canal de ocupação/material), e voxelização de uma malha
  (`fromMesh`, `cellSize`) com o voxelizador existente; sha256 e proveniência como os
  outros ativos; limite de células declarado (`maxCells`).
- **V.2 Objeto:** `<object3D primitive="voxels" voxels=IDREF cellSize=… palette=…>` com
  transformação, material por índice de paleta (cor, emissão, rugosidade), e `surface=
  "blocks|smooth"` (V1 só `blocks`).
- **V.3 Superfície:** extração das faces expostas por greedy meshing determinístico (quads
  com cor por face) numa malha que o raster e o traçador já desenham; recomputada só
  quando a grade muda (revisão); custo e memória orçados (`maxMemoryMiB`).
- **V.4 Física:** corpo rígido com massa = células × densidade × cellSize³ e colisor de
  caixas fundidas (ou casco), `rigidBody` como nos outros objetos; `crater@source` sobre um
  objeto de voxels remove as células dentro do bowl (destruição) e os componentes conexos
  que se separam viram corpos próprios com a sua massa; `fracture@source` corta pela
  partição em células.
- **V.5 Tempo:** sequências de voxels (`.vox` animado ou `srvseq` de ocupação) e cache por
  revisão, com seek e replay iguais ao bit.
- V2 (depois): raymarching DDA direto na grade esparsa no traçador (grades grandes), superfície
  `smooth` (marching cubes), edição por campo (adicionar/remover células por volume), LOD.

Oráculos de aceitação: importação `.vox` de ficheiros conhecidos (contagem de células, paleta,
posições) a 100%; greedy meshing de um bloco sólido n³ dá exatamente 6 quads e de um bloco
com um furo passante dá o número analítico; volume e massa conservados a 1e-12; o objeto de
voxels cai e repousa onde uma caixa igual repousa, a 1e-6; a cratera remove exatamente as
células cujo centro está dentro do bowl e a massa dos fragmentos destacados soma a das
células removidas da peça principal; render golden por hash do bloco e da cena de impacto
sobre uma parede de voxels; determinismo (seek, replay, 1/2/8 threads); custo por quadro
medido para 1 M de células (meshing e desenho) com orçamento declarado.

Esquema: `voxelAssetType`, `object3D@primitive="voxels"` e atributos, regras VOX1–VOXn
(versão 1.3, ativo existente, cellSize > 0, limite de células, paleta coerente, exclusões),
corpus, SREP com secção própria, ledger.

Divisão: Saturno (V.1 ativo e importador `.vox`, esquema, regras, corpus, SREP);
Mercurio (V.3 meshing e render, goldens, custo); Netuno (V.4 física, destruição por
cratera, componentes conexos, V.5 sequências e cache). Ordem: notas de desenho dos três →
V.1 → V.3 → V.4 → V.5 → V2 por decisão. Cena de aceitação nova: `impact-voxels.scene.xml`,
a rocha a atingir uma parede de voxels que se destrói, em HD e em pixel art.

### Fase 4 — Render à altura (2–3 semanas) → 93

| # | Tarefa | Onde | Nota |
|---|---|---|---|
| 4.1 | Espalhamento múltiplo em volumes: caminhos dentro do meio com amostragem por colisão nula, usando o máximo por bloco esparso como majorante | `volume.wgsl`, `pathtrace.wgsl` | Atributo `medium@scatterBounces`, padrão 1 (igual a hoje) |
| 4.2 | Desfoque de movimento de volumes pelo canal de velocidade | `volume.wgsl` | |
| 4.3 | Sombra através de água: raio de sombra atenuado pela transmissão em vez de bloqueado, para o fundo submerso receber luz | `pathtrace.wgsl` | Hipótese do fundo preto na cena original; verificar primeiro |
| 4.4 | Absorção por profundidade e espuma como mistura de albedo na superfície | materiais, `three.wgsl` | `attenuationColor/Distance` já existem |
| 4.5 | Névoa de respingo como volume: rasterizar partículas de spray em grade esparsa | `crates/sr-volume` | |
| 4.6 | Guias extras para o denoiser (albedo e normal do impacto refletido) e amostragem adaptativa por variância em cada tile | `pathtrace.rs` | Mantém determinismo por quadro |
| 4.7 | Tesselação adaptativa do terreno perto da cratera, com deslocamento | `crates/sr-3d/src/crater.rs`, `terrain.rs` | Hoje a cratera não refina a malha |

Critério de aceite:

- Teste de fornalha branca (meio com albedo 1 sob luz uniforme não ganha nem perde energia).
- Comparação de espalhamento múltiplo contra referência de força bruta em caso pequeno.
- Métrica de cintilação entre quadros consecutivos abaixo de limiar definido na Fase 0.
- Tempo por quadro UHD medido e dentro do orçamento acordado.

### Fase 5 — Validação e entrega (1–2 semanas) → 100

- Sequência completa 3840×2160 em modo estrito, codificada, com adaptador, configuração,
  memória e tempo registrados.
- Revisão temporal e de aparência quadro a quadro, com defeitos listados e resolvidos.
- Todos os atributos novos com XSD, Schematron, regra Rust, fixtures e seção no SREP;
  inventário exato atualizado.
- Suíte completa do workspace, MSRV, Clippy, formatação e build de release.
- Ledger com `goal_complete: true` somente quando todos os itens acima passarem.

## 5. Ordem, dependências e frentes

```
F0 ──► F1 (solvers) ───────────────┐
  └──► F2.0 agendador ─► F2.1–2.4 ─┼─► F3 ─► F5
  └──► F4.3, F4.6 (independentes) ─┘         ▲
                         F4.1, F4.2, F4.5, F4.7 ┘
```

- F2.0 (agendador) é o caminho crítico: F2.1–2.4 e toda a Fase 3 dependem dele.
- F1 e F4.3/F4.6 não dependem de F2 e podem andar em paralelo.
- F3.1 (líquido) depende de F1.3–1.6 (módulo MAC comum e solver rápido).
- F4.5 depende de F3.1; F4.7 serve a F3.3.

Três frentes possíveis: **solvers** (F1, F3.1, F3.2), **acoplamento** (F2, F3.3–3.5) e
**render** (F0.4–0.5, F4). Em uma frente só: cerca de 3 a 4 meses. Em três: 6 a 8 semanas.

## 6. Casos de validação

| Caso | Solver | Referência | Métrica |
|---|---|---|---|
| Lago em repouso com fundo irregular | Oceano | Audusse et al. 2004 | Velocidade permanece zero |
| Velocidade de onda longa | Oceano | c = √(g·h) | Erro de tempo de chegada |
| Fundo que sobe (geração de tsunami) | Oceano acoplado | Solução linear de águas rasas | Amplitude e conservação de volume |
| Rompimento de barragem | Líquido 3D | Martin & Moyce 1952 | Posição da frente no tempo |
| Entrada de esfera na água | Líquido + rígido | Experimentos clássicos de cavidade e jato de Worthington | Profundidade de fechamento da cavidade |
| Explosão pontual | Choque | Sedov–Taylor, R ∝ t^(2/5) | Expoente do raio da frente |
| Escala de cratera | Contato → cratera | Leis de escala π (Holsapple 1993) | Raio em função da energia |
| Cortina de ejetos | Contato → partículas | Modelo Z de Maxwell | Ângulo de lançamento e lei de velocidade por raio |
| Pluma térmica | Fumaça | Morton, Taylor & Turner 1956 | Lei de crescimento do raio com a altura |
| Fornalha branca | Render de volumes | Conservação de energia | Radiância de saída igual à de entrada |

As referências devem ser conferidas e citadas com DOI no SREP ao implementar cada caso.

## 7. Mudanças de esquema previstas

| Elemento | Atributo ou filho novo | Padrão | Fase |
|---|---|---|---|
| `pyro` | `solver`, `advection` | Comportamento atual | 1 |
| `ocean` | `order` | 1 | 1 |
| `ocean` | `colliders`, `density` | Ausente | 2 |
| `crater` | `source` e constantes de material | Ausente (modo autoral atual) | 2 |
| `burst`, `pyroSource`, `pyroImpulse`, `fracture` | `on="contact"`, `body`, fração de energia | Ausente (modo por tempo atual) | 2 |
| `ocean` | filho de região líquida 3D | Ausente | 3 |
| `pyro` / SRVOL | segundo canal de densidade | Ausente | 3 |
| `medium` | `scatterBounces` | 1 | 4 |

Nomes são proposta; fecham na revisão do SREP.

## 8. Riscos

| Risco | Efeito | Mitigação |
|---|---|---|
| Agendador conjunto quebra replay ou orçamento de checkpoints | Bloqueia as Fases 2 e 3 | Fazer primeiro, com o caso mais simples (oceano ← fundo), antes de qualquer mão dupla |
| Líquido 3D custa mais que o previsto | Fase 3 estoura o prazo | Domínio local pequeno; meta de resolução definida após F1; etapa A do choque não depende dele |
| Paralelismo altera bits de cenas existentes | Quebra caches e testes | Etapa 1.2 mantém reduções seriais; mudanças numéricas só por atributo |
| Dois agentes editando os mesmos arquivos | Conflitos e evidência invalidada | Divisão de frentes (D3); trabalho em worktree e branch próprios |
| Esquema muda mais rápido que o SREP | Rejeição upstream | Atributos novos entram no SREP na mesma mudança do código |
| Espalhamento múltiplo torna o quadro UHD inviável | Fase 4 não fecha orçamento | Limite de saltos por atributo; aproximação barata para preview |

## 9. Decisões de execução

O usuário delegou as decisões de execução ao coordenador (Urano) em 2026-10-03. Decisões
tomadas até aqui:

- **D1 — Escopo do 100.** Causal e visualmente convincente, verificado contra os casos
  canônicos da seção 6. Previsão científica em nível de hidrocódigo fica fora.
- **D2 — Solvers na GPU.** CPU paralela é a referência determinística. Backend de GPU só
  depois da Fase 1, se as metas de tempo não fecharem, e sempre comparado contra a CPU.
- **D3 — Coordenação.** Saturno: fumaça (1.1–1.9), branch `phase1/pyro-perf`. Netuno:
  oceano (1.10–1.12), branch `phase1/ocean-order2`. Integração por Urano em
  `phase1/integration`. Desde 2026-10-03 os três são os únicos agentes ativos no
  repositório (os demais pararam por ordem do usuário), e `cinematic-impact-checkpoint`
  está parado em `60a458c`. Por isso as tarefas 1.13 e 1.14 passam para esta equipe. Mercurio
  (modelo Sonnet, branch `phase1/render`) entrou para a frente de render: itens 0.1, 0.2,
  0.4 e 0.5 da Fase 0 e tarefas 1.13 e 1.14. O ledger e o SREP passam a ser atualizados na
  integração, por Urano. Em 2026-10-04, com o oceano concluído, Netuno assumiu o esquema
  e a ligação no avaliador de `pyro@solver`, para encurtar a fila do Saturno, que fica com
  a estimativa de memória, a advecção MacCormack e o esquema de `pyro@advection`.
- **D4 — Orçamentos provisórios.** Render: até 60 s por quadro UHD em qualidade final na
  RTX 6000 Ada. Simulação: até 30 min para a sequência de 6 s. Revisar com os números da
  tarefa 1.1 e da Fase 0.
- **D5 — Fluxo de commits.** Commits locais por tarefa nos branches de cada frente; merge
  em `phase1/integration` só depois de revisão e testes rodados pelo coordenador. Nenhum
  push para o remoto sem pedido explícito do usuário.
- **D6 — Mudanças numéricas.** Onda 1 da fumaça é bit a bit idêntica ao commit base.
  Multigrid, MacCormack e reduções paralelas entram na onda 2, por atributo com padrão
  igual ao comportamento atual.

## 10. Primeiro passo

Fase 0 completa e, em paralelo, F2.0 + F2.1 no caso mais simples: a cratera move a água.
É a menor mudança que troca um efeito autoral por um efeito simulado e exercita o
agendador que todo o resto precisa.
