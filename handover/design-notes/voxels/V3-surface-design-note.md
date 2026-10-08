# V.3 Superfície de voxels: nota de projeto final

Base: main d87fc65, worktree mercurio-gl. Nenhum código foi construído ou executado em Rust, e não houve GPU.

Convenções de evidência usadas na nota:
- **ref-medido**: contagem de um mesher de referência em Python, que implementa exatamente a regra da seção 1. Ele não é o mesher em Rust.
- **derivado**: aritmética sobre o código lido.
- **estimado**: tempo calculado com custos unitários assumidos, nunca medido.
- **não verificado**: não li ou não rodei.

Scripts de referência: `/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad/v3_ref.py` (oráculos e hashes) e `greedy_count.py` (tabela de 1 M de células).

## 0. Correções ao enunciado

1. O tipo é `sr_3d::occupancy::Occupancy` (`crates/sr-3d/src/lib.rs:32`, módulo `pub`, sem re-export na raiz).
2. `revision()` é um contador por instância, não um hash (`occupancy.rs:244`, `:273-276`). `Clone` está derivado (`:64`). Dois clones editados de formas diferentes chegam ambos a r+1, e um clone antigo restaurado tem revision menor. A chave de cache precisa, portanto, de `(lineage, revision)`.
   - A identidade por conteúdo é `fingerprint()` (`:306-322`). Ela é O(conteúdo): cerca de 1 MB de bytes hasheados para 1 M de células densas, e cerca de 524 MB no pior caso esparso.
3. A paleta já mora dentro de `Occupancy`: 256 entradas RGBA, índice c em `colors[c]`, com `palette_revision` próprio (`:39-61`, `:183-224`). Recolorir não muda `revision` nem `fingerprint`.
4. `changed_bricks_since(rev)` já existe (`:284`). Ele é O(B) e inclui bricks esvaziados até `compact`. `bricks()` omite bricks com `filled==0` (`:279-281`). Um brick que aparece em um e não no outro significa "esvaziado", e o mesher depende dessa assimetria (o teste `occupancy.rs:92-109` a fixa).
5. Não existe getter de brick por chave. `get()` é uma busca em BTreeMap por célula, então o mesher nunca o usa por célula.
6. Os bricks são de 8³. Cortar quads na borda do brick daria 24 quads para um bloco 16³ alinhado. O oráculo "n³ sólido = 6 quads" obriga a fundir através de bricks (1.4).

## 1. Superfície

### 1.1 Onde mora

- O mesher é uma função pura de CPU, em um módulo novo `sr_3d::voxel`. Não depende de GPU, sr-eval nem sr-sim, então os testes rodam iguais em GPU real e no adaptador de software.
- **Entrada:**
  - `&Occupancy`;
  - tabela de classes `class[256]`, com 0 para vazio. Classe é equivalência de aparência resolvida (cor mais assinatura de material). O rótulo canônico é o menor índice da classe;
  - bit `see_through` por classe.
- **Saída:** lista compacta de quads, independente da paleta e de `cellSize`. São 16 B por quad:
  - `u0` i32, `v0` i32, `w` u16, `h` u16, classe u8 e 3 B de padding;
  - eixo, sinal e camada vêm da chave da lista.
- O renderer expande a lista em `sr_3d::Vertex` no upload e aplica cores e grupos.
- Nenhum shader, layout de vértice ou pipeline é tocado.

### 1.2 Regra de exposição

A face da célula A em direção a B (só vizinhança por face) é emitida se, e somente se:
1. B é vazio; ou
2. A é opaca e B é see-through; ou
3. ambas são see-through, de classes diferentes, e `classe(A) < classe(B)`.

Isso dá um único dono por interface e nenhuma face coincidente. Opaco com opaco nunca emite. Vidro contra opaco não emite pelo lado do vidro, porque a face do opaco já cobre. Faces entre duas células da mesma classe de vidro são omitidas, para o vidro continuar um casco fechado.

- see-through é transmissão > 0 ou alpha mode blend, na aparência resolvida.
- Sem classes see-through, a regra é exatamente "vizinho vazio".
- Domino de duas células: 10 quads se opaco/opaco, 11 se opaco/vidro (ref-medido).
- Limitação a documentar: com a regra do dono único, a interface vidro-vidro de classes diferentes é um quad de uma face só. Com `cull_mode Back` (`three.rs:1113-1117`), ela some vista do lado da classe maior no raster. O comportamento no tracer (`pathtrace.wgsl:693-714`, rastreio de meio por `in_sigma`) **não verificado**. Ver decisão 3.

### 1.3 Regra de fusão

- Duas faces se fundem se, e somente se, têm mesmo eixo, sinal, camada, **classe** e formam um retângulo contíguo ainda não consumido.
- Cor, material e emissão são funções da classe, então nada mais entra na regra.
- Sem `@material` no objeto e sem paleta de IDREFS, a tabela de classes é a identidade, ou seja, classe = índice. Com um `@material` único, todos os índices colapsam em uma classe e o cubo dá 6 quads.
  - Sem a tabela, 16 índices aleatórios em 100³ dão 53.356 quads para algo que parece uniforme (ref-medido).
- A chave de geometria usa o hash da partição em classes, não as cores. Recolorir sem mudar a partição só reexpande, nunca remalha. Se a recoloração funde ou divide classes (ou uma cor de material de paleta é animada e cruza essa fronteira), há remalha. Isso deve ser documentado.

### 1.4 Ordem de varredura, desempate e saída canônica

- **Eixos cíclicos.** Para o eixo a, u = (a+1)%3 e v = (a+2)%3, de modo que u×v = +e_a.
- **Plano de uma face.** Face + da célula c fica no plano c_a+1. Face − fica no plano c_a. O dono é c.
- **Ordem de varredura:**
  - eixo 0..2, depois sinal (−, +), depois plano crescente, depois v crescente (externo) e u crescente (interno);
  - no primeiro ponto não consumido (u0,v0) de classe i, `w` é a maior corrida em +u de células não consumidas de classe i;
  - `h` cresce em +v enquanto a linha inteira [u0, u0+w) tiver classe i e estiver não consumida;
  - emite o quad e marca as células. `w` e `h` são limitados a 65.535, e acima disso o quad é dividido.
- **Desempate:** não há, a regra de largura primeiro é determinística. A ordem importa para a contagem de formas gerais. Exemplo: uma célula em v0 com uma linha de 3 em v1 dá 3 quads com largura-primeiro e 2 com altura-primeiro (u externo). A ordem fica portanto fixada pelo hash dourado, não por igualdade de contagem em sólidos simétricos.
- **Ordem canônica de saída:** (eixo, sinal, plano, v0, u0). A varredura já emite nessa ordem.
- A fusão é por plano e função só da máscara de faces do plano. Por isso o cache por plano é igual a uma remalha completa.
- Floats saem de coordenadas inteiras por uma única função, de modo que coordenadas iguais são bits iguais entre quads vizinhos. Não há HashMap iterado nem dependência de thread.

### 1.5 Bricks, leitura esparsa e custo

- Bricks são só fonte de dados e rastreio de sujeira. Quads atravessam bricks, chaves negativas e origem não alinhada.
- **Máscara esparsa por tiles.** A máscara de uma fatia (eixo, sinal, camada) é um conjunto de tiles 8×8 alinhados a bricks, onde um tile ausente lê como 0.
  - Não há máscara densa sobre a caixa envolvente. O custo é proporcional aos bricks presentes.
  - A varredura global passa pelas linhas v dos tiles presentes em ordem crescente e consome células através das bordas de tiles.
- **Exposição por bitboard.** Cada brick vira um bitboard de 512 bits (8 u64). A exposição das 6 direções sai de shifts e ANDs por palavra, e as faces expostas são extraídas com ctz.
  - Bricks cheios com os 6 vizinhos cheios são descartados em O(1) pelo popcount do próprio bitboard. Não é preciso um getter `filled`.
- Leitura do grid: 1 lookup por brick mais 6 vizinhos, nunca `get()` por célula. Enquanto `Occupancy` não tiver `brick(key)`, o mesher coleta `bricks()` em um índice local por remalha (O(B), aceitável até uns 1e5 bricks, **não medido**).
- **Por que não máscara densa.** Para a casca de 2 células (989.240 células, ref-medido) a caixa é 398³. Uma máscara densa leria 3 × 399 × 398² ≈ 1,9e8 pares, contra 12.101 bricks (5,9 MiB no Occupancy, cerca de 82 células por brick) na leitura esparsa.

### 1.6 Layout de saída

Um quad vira 4 vértices `sr_3d::Vertex` (96 B cada, `lib.rs:49-75`) e 6 índices u32 (único formato, `three.rs:2096/2108/2156`). São 408 B por quad, sem compartilhamento de vértices.

| Campo | Valor |
|---|---|
| pos | coordenadas inteiras de célula, em f32 (exato abaixo de 2^24). Origem no canto da caixa ocupada |
| escala | `cellSize` vai em `Draw3.model` como escala uniforme. O mesh independe de `cellSize` e da pose |
| normal | ±e_a, unitária, igual nos 4 vértices |
| uv | (u,v) em unidades de célula. Fusão não muda a textura por célula sob sampler repeat |
| tangent | +u, com w = +1 para sinal + e −1 para sinal −. Assim cross(n,t)·w = +v, que é a convenção de `compute_tangents` (`lib.rs:392-423`: w = sinal de n×t·bit) |
| map_uv | zero |
| color | RGBA linear, alpha 1, igual nos 4 cantos (seção 2) |
| ordem dos cantos | (u0,v0), (u0+w,v0), (u0+w,v0+h), (u0,v0+h) |
| índices, sinal + | (0,1,2), (0,2,3) |
| índices, sinal − | (0,3,2), (0,2,1) |

- Não se chama `compute_normals` nem `compute_tangents`, que alocam por vértice.
- **Contrato de enrolamento.** Frente é CCW com `(b−a)×(c−a)` apontando para fora, e `doubleSided` desligado faz culling de verso. A rotação de eixos de cena (x,−z,y) tem determinante +1 (derivado) e não inverte o enrolamento. O oráculo 3.4 testa isso, sem presumir.
- **Contrato de coordenadas** com o colisor: célula [i,j,k] preenche [i,j,k]·cellSize até [i+1,j+1,k+1]·cellSize, sem meia célula (`physics3d.rs:51-57`, `:656-664`). Portanto a AABB do mesh é igual à do colisor.
- **Peças.** `Draw3` não tem sub-intervalo, e os três `draw_indexed(0..m.count)` desenham o `MeshGpu` inteiro (`three.rs:2097/2109/2157`). Um grupo é dividido em peças por prefixo da ordem canônica, com no máximo ⌊max_buffer_size/384⌋ quads por peça (699.050 se o teto for 256 MiB). Cada peça é um `upload_mesh`, ou seja, dois buffers e uma cópia de CPU. Peças são faixas da ordem canônica, não regiões espaciais. Os pisos de limites (`gpu.rs:201-215` usa o maior entre o padrão downlevel e o adaptador) são pisos, não tetos de GPU real.
- Grupo ou peça vazia não gera `Draw3` (como clay, `render_three.rs:668`).

### 1.7 Recalcular só quando a revision muda

- **Cache de geometria.** Vive no Program do sr-eval (Mutex, como `mesh_sequence_cache`, `program.rs:505`).
  - Guarda `Arc<[Quad]>` por (eixo, sinal, camada).
  - A chave é `(lineage, revision, hash da partição, hash do see_through)`.
  - Se a lineage difere ou a revision é menor que a construída, faz remalha completa.
  - Frame sem mudança custa O(1) no evaluator: uma comparação de revision e um clone de `Arc`.
- **Chave do mesh de GPU (string).** Não leva id de nó, para que dois objetos com o mesmo conteúdo compartilhem um `Arc<MeshGpu>`:
  `voxels|{lineage}|{revision}|{hash_particao}|{hash_see}|{assinatura de paleta}|{espaço de trabalho}|{grupo}|{peça}`
  - O raster funde draws de mesmo `Arc` e material em um `draw_indexed` instanciado (`three.rs:1903-1921`).
  - O tracer constrói um BVH-protótipo por `Arc` (`pathtrace/instances.rs:8-30`).
  - A chave é O(1) por frame. Não copiar o padrão de clay, que monta uma string de Debug por frame (`render_three.rs:663`).
- **Despejo.** `meshes` nunca despeja chaves de primitivas (`three.rs:503`). É preciso um mapa por nó das chaves anteriores, no padrão de `clay_keys` (`render.rs:351`, `render_three.rs:660-679`), removendo as chaves antigas na mudança.
- Recolorir (`palette_revision`) reexpande e reenvia, mas não remalha.

### 1.8 Remalha incremental

- Para cada brick de `changed_bricks_since(rev_construída)` (inclui esvaziados), as camadas donas sujas por eixo são:
  - sinal +: [8b−1, 8b+7]; sinal −: [8b, 8b+8];
  - isso dá 18 por eixo e 54 fatias por brick, compartilhadas entre bricks vizinhos.
  - Derivação: uma célula na camada c afeta faces de dono nas camadas c−1, c e c+1.
- Cada fatia suja é remalhada inteira, e as demais reaproveitam seu `Arc`. Se mais da metade das fatias estiver suja, faz remalha completa.
- **Custo (derivado).** Edição de um brick num 100³ sólido: 54 fatias de até 1e4 células = 5,4e5 testes de máscara, contra 6e6 numa remalha completa (cerca de 11 vezes menos). A expansão e o upload são O(Q) e continuam sendo refeitos por inteiro, porque `upload_mesh` substitui o buffer (`three.rs:1183-1210`). O ganho é só de CPU de mesher.
- **Oráculo do passo:** o incremental é byte-idêntico a uma remalha completa de um grid fresco de mesmo conteúdo, comparado fatia a fatia.
- **Cerca de compactação:** o mesher precisa ter consumido `changed_bricks_since` antes de o dono chamar `compact(rev)`. Caso contrário, quads de um brick esvaziado sobrevivem. A regra é "a revision construída pelo mesher é a cerca do `compact`" (ver API).
- **Recomendação:** a V1 entrega remalha completa por `(lineage, revision)` e o cache por fatia entra num commit separado, gateado pelo oráculo acima.

### 1.9 Orçamento de memória

- **Atributo:** `surfaceMemoryMiB` em `object3D primitive="voxels"`.
  - Inteiro positivo, padrão 128, máximo 4096, no estilo do oceano (`schema/scene-render-1.1.xsd:4474`).
  - O asset mantém `maxCells` e `maxMemoryMiB` próprios para o grid.
  - Memória do grid conta bricks (512 B cada), não células: 1,07 MiB para 100³ sólido e 488 MiB para 1 M de células isoladas, uma por brick.
- **Custo de pico por quad: 1.240 B**, que é 16 (lista compacta) + 408 (vetores do construtor) + 408 (clone `MeshGpu.cpu`, `three.rs:1206`) + 408 (buffers de GPU). Residente depois do upload: 832 B.
- **Quads admitidos** (⌊MiB·2^20 / 1.240⌋): 108.240 (128 MiB), 216.480 (256), 865.920 (1024) e 3.463.683 (4096).
- **Aborto corrente** em ordem canônica, com aritmética verificada como em `ocean/surface.rs:3-14`.
  - Aborta quando Q×1.240 > orçamento. Para o tabuleiro de xadrez com 128 MiB isso ocorre no quad 108.241 de 6.001.128, ou seja, em 1,8% do trabalho.
  - Contar faces expostas F por popcount dá só um majorante de Q, usado para dimensionar scratch e compor a mensagem de erro. A contagem F não pode recusar sozinha: F×1.240 recusaria erradamente a casca, cuja necessidade real é 748 MiB.
- **Falha:** no evaluator, `g.fail("{id}: voxel surface exceeds memory budget (...)")` (`eval.rs:318-343`, `render.rs:3193-3199`). O erro nomeia a causa e o `failed(id)` suprime o "produced no surface" duplicado. Não se emite `Draw3`, nem mesmo parcial.
- **Teto de dispositivo:** por peça, V×96 e I×4 contra `max_buffer_size`, no padrão de `render_three.rs:696-702`. Erro "{id}: voxel surface exceeds device buffer limits".
- **Tracer:** 384 B por triângulo em um único binding de storage. Se não couber, `limit_note` rasteriza o passe com uma nota `unsupported` (`pathtrace.rs:174-214`, `render_three.rs:2909-2921`). No piso de 128 MiB isso é 349.525 triângulos, ou 174.762 quads.
- **Teste de CLI:** caso "voxel surface" em `solver_failures.rs`, com `surfaceMemoryMiB="1"` e o texto da causa exigido.

### 1.10 Tabela para 1 M de células

Todas as linhas usam uma só classe. As contagens são ref-medidas pelo mesher Python. A casca é uma esfera oca com 2 células de espessura (centro em meia célula, R ≤ 199,5 e R > 197,5, 989.240 células, caixa 398³). A esfera sólida tem r=62 (1.000.608 células). Os bytes e os tempos são derivados ou estimados.

| | Cubo 100³ | Esfera sólida r=62 | Casca 2 células | Tabuleiro de xadrez 126³/2 |
|---|---|---|---|---|
| Células | 1.000.000 | 1.000.608 | 989.240 | 1.000.188 |
| Faces expostas F | 60.000 | 72.624 | 1.484.904 | 6.001.128 |
| Quads Q | **6** | 31.422 | 632.418 | 6.001.128 |
| Vértices (4Q) | 24 | 125.688 | 2.529.672 | 24.004.512 |
| Índices (6Q) | 36 | 188.532 | 3.794.508 | 36.006.768 |
| Mesh (408 B/quad) | 2.448 B | 12,2 MiB | 246,1 MiB (buffer de vértices 231,6 MiB) | 2.335 MiB (buffer de vértices 2.198 MiB, 8,6× o teto de 256 MiB) |
| Lista compacta (16 B/quad) | 96 B | 0,48 MiB | 9,7 MiB | 91,6 MiB |
| Pico (1.240 B/quad) | 7,3 KiB | 37,2 MiB | 747,9 MiB | 7.096,7 MiB |
| Residente (832 B/quad) | 5 KiB | 24,9 MiB | 501,8 MiB | 4.762 MiB |
| `surfaceMemoryMiB` necessário | 1 | cabe no padrão 128 | 748 (o autor declara 1024) | recusado em qualquer valor legal (> 4096) |
| Geometria do tracer (2 tri/quad × 384 B) | 4,5 KiB | 23,0 MiB | 463,2 MiB (excede o piso de 128 MiB, cai para raster com nota) | 4.395 MiB, nunca cabe |
| CPU, 1 remalha completa (estimado) | 1,5 a 3 ms | cerca de 15 ms | 0,13 a 0,3 s | aborta em cerca de 35 ms |

Derivação da CPU, com custos unitários **assumidos**:
- 0,4 µs por brick (bitboard mais 7 lookups);
- 10 ns por face exposta (escrita de máscara mais visita gulosa);
- 180 ns por quad (expansão de 408 B mais duas cópias de 408 B em `upload_mesh`).

Exemplo da casca: 12.101 bricks × 0,4 µs ≈ 5 ms, mais 1,485 M faces × 10 ns ≈ 15 ms, mais 632.418 quads × 180 ns ≈ 114 ms, ou seja, cerca de 0,13 s. A expansão e o upload dominam. Nenhum tempo foi medido. O dado medido mais próximo é a construção do colisor, 166 ms para 1 M de células (texto do commit d87fc65, não relido). Proponho a estatística `voxel_mesh_seconds` em `--stats` e uma sonda antes de prometer qualquer número.

**Custo por frame com grid inalterado:**
- **Evaluator:** O(1).
- **Compositor:** um u64 em `node_hash`, adicionado só quando o campo é `Some` (`render.rs:1743-1778`). Um asset `voxelAsset` novo precisa ser listado ao lado de `Image` e do volume estático na classificação `timed` (`render.rs:3238-3239`), senão é re-hasheado a cada frame.
- **Renderer:** uma busca de string e um clone de `Arc` por (grupo, peça), mais um `ObjectU` de 256 B por draw e por view de sombra. A chave do mesh de GPU deve ser O(1).
- **GPU no raster:** 2Q triângulos × passes (1 principal, +1 de pré-passe se AO, SSR ou sombras de contato, e uma por view de sombra: 4 cascatas para luz direcional em câmera perspectiva e 6 para luz pontual). O passe principal não tem culling de frustum (`three.rs:2101-2111`). Só o passe de sombra pula draws fora do volume.
  - Esfera: 62.844 triângulos por passe. Casca: 1,26 M por passe.
- **Tracer:** reconstrói triângulos, BVH, empacota e sobe os buffers em todo frame (`three.rs:1493`; `pathtrace.rs:1360-1462`). O cache de revision só poupa o mesher.
  - Esfera: 62.844 triângulos, cerca de 4,6e6 visitas de BVH (5·log2(T/2,5)·T), 23 MiB por frame.
  - Casca: 1,26 M triângulos, cerca de 1,2e8 visitas, 463 MiB por frame, e só se o binding couber.
  - Não prometo tempo em milissegundos sem `tools/probe_render.py` (`pt_assemble_seconds`, `pt_bvh_seconds`, `pt_pack_seconds`) sobre um fixture de voxels.

## 2. Cor, material e emissão por face

### 2.1 Cor: cor de vértice, sem mexer em shader

- Ambos os caminhos multiplicam a cor de vértice na cor base. No raster, `base = mat.base_color * i.color` (`three.wgsl:428-431`). No tracer, `m.base = m.base * base_texel * color` (`pathtrace.wgsl:659-661`).
- A cor de vértice é f32 linear e **não** é convertida para o espaço de trabalho (`lib.rs:60`). O renderer converte a cor da paleta na expansão, com `lin_srgb` (`render_three.rs:483-487`). Por isso a chave do mesh de GPU inclui o espaço de trabalho.
  - O oráculo compara um voxel com uma caixa de material de documento em espaço não-sRGB.
- Alpha de vértice é o alpha da paleta. Em draw opaco ele é ignorado no tracer, e no raster só vale em mask ou blend.

### 2.2 Material por face: grupos por assinatura não-cor

- Resolução por índice: `palette[i]` > material do objeto > cor do arquivo.
  - Índices de cor de arquivo ficam em um grupo cuja assinatura é (kind, roughness, metallic, transmission, ior, alphaMode, opacidade). O material do draw tem `base_color` branco e a cor vai no vértice. O material padrão do objeto tem `base_color` 0,8 e escureceria a cor (`render_three.rs:1420-1424`).
  - Índices que resolvem para material de documento usam esse material inteiro (mapas incluídos), com cor de vértice branca.
  - Um `@material` no objeto faz todos os índices não sobrescritos virarem um grupo.
- Um `Draw3` por (grupo, peça), com `model = pose × escala(cellSize)`. Nada muda no tracer: um `PtMat` de 288 B por draw, ou seja, até 255 × 288 = 73.440 B.
- Custo de draws: 256 B de `ObjectU` por draw e por view, 512 B por slot de material distinto, e um par de buffers por (grupo, peça).
  - No máximo 255 grupos. Não li as paletas reais do importador, então a hipótese de "poucos grupos" é uma suposição. `voxel_groups` e `voxel_quads` entram nas estatísticas.
- Transmissão, ior e blend são escalares por draw. Todo grupo de vidro ou blend é um draw próprio e muda a classe do draw (`three.rs:1820-1831`). Qualquer draw transmissivo seleciona o pipeline de água do tracer para a cena inteira (`pathtrace.rs:1131-1135`), como já acontece com qualquer vidro. Cenas sem vidro não mudam.
- Blend e transmissivo ordenam por draw (centro dos bounds), não por triângulo. Vidro só fica correto em objeto convexo ou de camada única.

### 2.3 Emissão por voxel

Nenhum shader multiplica a emissão pela cor de vértice (`three.wgsl:623-625`, `pathtrace.wgsl:664-665`, `:700`). Exceção: material unlit, em que a saída é `base.rgb`, que inclui a cor de vértice (`three.wgsl:551-556`, `pathtrace.wgsl:665`). Não há nenhum campo de vértice de "flag emissiva" lido por shader, e criar um exigiria editar shader, então isso fica para a V2.

| Opção | Mecanismo | Limites |
|---|---|---|
| E1 | Um grupo por (cor, força) emissiva distinta; `emissive = cor × força` no material do draw | Exato. Um draw por emissor distinto |
| E2 | Um grupo unlit; cor de vértice = cor × força | Um draw para todos os emissores puros. O voxel não reflete nem recebe luz, e no tracer o caminho termina no hit. Brilho acima de 1 no raster depende do formato de alvo (não verificado) |

- Emissor **não ilumina vizinhos** no raster (sem GI nem iluminação por emissor, só `scene.lights`).
- No tracer **não há NEE em direção a triângulos emissivos** (o laço só percorre `scene.lights`, `pathtrace.rs:445-477`, `pathtrace.wgsl:718-736`). Um voxel ilumina o entorno só quando um caminho amostrado por BSDF o atinge.
  - Ordem de grandeza (derivada): uma célula de lado 1 a 3 células de distância, de frente, tem probabilidade de cerca de A/(π d²) = 1/(9π) ≈ 3,5% por amostra difusa.
  - O termo de emissão não é limitado (`pathtrace.wgsl:700`), então emissor pequeno gera ruído e fireflies.
  - Documentar que luz explícita é o caminho confiável.

### 2.4 Shaders e pipelines

Nada muda em `three.wgsl`, `three_types.wgsl`, `pathtrace.wgsl`, no layout de vértice (96 B) ou na criação de pipelines. O trabalho é de montagem de mesh e de `Draw3`:
- sr-3d: módulo `voxel`;
- sr-eval: módulo `voxels` (orçamento, cache, erro), a seta `asset_ref` para `primitive=="voxels"` (`program.rs:1356`) e `FrameNode.voxels` para grids de simulação, no estilo de `fracture` (`eval.rs:204-231`);
- sr-gpu `render_three.rs`: um ramo novo `primitive=="voxels"` antes do despacho genérico (`render_three.rs:1386-1446`, onde um tipo desconhecido dá erro);
- sr-model: `voxels` no enum de primitivas, `surfaceMemoryMiB` e as regras, sem tocar `crates/sr-sim`.

## 3. Oráculos

Tudo roda sem GPU, salvo 3.7.

**3.1 Bloco sólido n³ dá exatamente 6 quads.**
- Casos: n = 1..12, 16, 17, 25, nas origens (0,0,0), (−3,5,−7) e (5,5,5). Também n=33 e 100 (este em grid de construção em bloco).
- ref-medido para n = 1..12, 16, 17, 25 nas três origens, com F = 6n².
- Equivale ao primitivo cuboide (24 vértices, 12 triângulos).

**3.2 Bloco n³ com furo passante quadrado h×h, quantidade analítica: 16 quads.**
- Condição: furo ao longo de um eixo, com margem de pelo menos 1 célula nos quatro lados.
- Faces expostas: 6n² − 2h² + 4hn. Para n=8, h=2 são 440.
- Contagem: 2 anéis de tampa × 4 + 4 lados externos × 1 + 4 paredes do furo × 1 = 16.
- Ordem de varredura que produz esse particionamento. Furo ao longo de z, n=8, h=2, a=3, tampas no eixo z com u=x e v=y, quatro quads por tampa, dados como (u0, v0, w, h):
  1. (0, 0, 8, 3), a barra superior;
  2. (0, 3, 3, 5), a barra esquerda, que desce até o fundo;
  3. (5, 3, 3, 5), a barra direita, idem;
  4. (3, 5, 2, 3), a peça inferior central.
  - As áreas somam 24+15+15+6 = 60 = 64 − 4 (conferido).
  - Em ordem canônica, os quads da tampa em plano 0 (sinal −) e plano 8 (sinal +) saem com (v0,u0) crescentes.
- ref-medido: 16 quads para (n,h) = (3,1), (4,2), (5,1), (5,3), (6,2), (6,4), (8,2), (8,4), (8,6), (9,3), (12,2), (12,6), (16,4), e para furo fora do centro com margem ≥ 1 (n=9, h=3, a=1). A contagem não depende da ordem de varredura nesses casos (largura-primeiro e altura-primeiro coincidem), mas a forma dos quads depende.
- Fora da condição: furo encostado na borda (margem 0) muda a contagem. Para n=9, h=3, a=0 dá 10 quads. O enunciado do oráculo é, portanto, "margem ≥ 1".

**3.3 Hashes dourados (os de blocos, do furo e da parede).**
- **Algoritmo:** FNV-1a 64 (offset `0xcbf29ce484222325`, primo `0x100000001b3`, o mesmo laço de `fingerprint`).
- **Serialização** da lista compacta canônica, por quad: eixo u8, sinal u8 (0 = −, 1 = +), plano i32 LE, u0 i32 LE, v0 i32 LE, w u16 LE, h u16 LE, classe u8. Ordem (eixo, sinal, plano, v0, u0).
- **Tabela de classes:** identidade (classe = índice).

| Fixture | Quads | FNV-1a 64 |
|---|---|---|
| Bloco 8³ na origem, índice 1 | 6 | `4a7c15d5ebc45cc8` |
| Furo: n=8, h=2, a=3, ao longo de z, índice 1 | 16 | `b61f3c69c0f98e99` |
| Parede x∈[0,24), y∈[0,3), z∈[0,16), índice = 1 + ((x/4 + z/2) mod 2) (divisão inteira), 1.008 faces | 124 | `5fdf69dec601e091` |

- A contagem da parede se deriva à mão: 96 (topo e base: 48 tiles de 4×2 por face × 2) + 16 (extremos x: 8 faixas × 2) + 12 (extremos z: 6 faixas × 2).
- Os valores vêm do mesher de referência. Eu os rodei, e uma reimplementação independente os reproduziu bit a bit. O mesher em Rust deve reproduzi-los, ou a regra escrita está errada.
- Um segundo hash por fixture cobre os bytes expandidos (`Vec<Vertex>` e `Vec<u32>`) para paleta fixa em sRGB. Ele fixa a conversão de cor, a tangente e o uv. O hash da lista compacta independe da paleta e do layout do vértice.
- O script de referência entra no repositório como oráculo independente.

**3.4 Conservação e estrutura.**
- Soma das áreas dos quads é igual a F, contada ingenuamente com `get()` por vizinho 6.
- Cobertura exata: um contador por (eixo, sinal, plano, u, v) é exatamente 1, e o conjunto coberto é igual ao conjunto ingênuo de faces expostas (ref-medido na parede).
- Cada quad tem dono ocupado de mesma classe e vizinho vazio, ou see-through conforme 1.2.
- Enrolamento `(b−a)×(c−a)·n > 0` nos dois triângulos de cada quad, normal unitária, tangente perpendicular com |w|=1, cor igual nos 4 cantos.
- Fechamento em aritmética inteira exata (grids só opacos): partindo cada quad nos vértices da malha, cada aresta unitária é usada um número par de vezes (2, ou 4 onde duas células se tocam só por aresta).
- A AABB do mesh é igual a [min chave, max chave + 1] × cellSize. Q ≤ F. Um tabuleiro 20³ dá exatamente 6 quads por célula.
- `size_of::<Vertex>() == 96` (nenhum teste hoje afirma isso).

**3.5 Determinismo e cache.**
- Mesmo conteúdo inserido em ordem embaralhada, ou por `from_cells` contra uma sequência de `set`, dá os mesmos hashes.
- Incremental sobre uma sequência aleatória de edições (com bricks esvaziados e bordas de brick) é byte-idêntico à remalha completa.
- Recolorir mantém o hash da lista compacta e não remalha (contador). Mudar a partição de classes remalha.
- Mesma revision devolve o mesmo `Arc` e uma nova revision despeja a chave antiga. Lineage diferente, ou revision menor, força remalha completa.

**3.6 Orçamento.**
- O tabuleiro de 1 M de células com orçamento no máximo é recusado com o texto da causa, abortando no quad 108.241 com 128 MiB.
- Um grid abaixo do teto desenha.
- Um grid vazio não gera `Draw3`.

**3.7 Identidade de cenas sem voxels** (capturar antes do primeiro commit de código).
- Hash dos fontes `three.wgsl`, `three_types.wgsl` e `pathtrace.wgsl`, e do fonte de água gerado.
- `git diff main -- crates/sr-gpu/src/*.wgsl` vazio.
- `node_hash` de nós existentes inalterado, com o termo novo só quando `Some`.
- Pixels de 2 ou 3 cenas dos exemplos no adaptador de software, contra a base. As falhas conhecidas do llvmpipe são a linha de base.
- Os oráculos de pixel dos voxels ficam secundários e só rodam com `SR_REQUIRE_GPU` (padrão de `three.rs:2503-2600`): bloco contra o primitivo cuboide, e voxel de paleta contra caixa de material de documento em sRGB e fora dele.

## 4. Ideias da V2

- **DDA direto na grade esparsa dentro do tracer:** sem malha por frame, sem BVH, sem 384 B por triângulo.
- **Superfície suave:** marching cubes ou dual contouring sobre a ocupação, com normais pelo gradiente.
- **LOD:** pirâmide de ocupação por maioria 2×, malhando níveis grossos longe.
- **Peças espaciais com atualização parcial:** malhas por região e `first_index/index_count` em `Draw3`, para cortar o upload por edição e permitir culling de sombra por peça.
- **BVH persistente do tracer** entre frames.
- **NEE para quads emissivos:** como lista de luzes de área, numa variante de shader.
- **Bandas de emissão:** textura de paleta com bandas por potência de 4, se um draw por emissor distinto explodir.
- **Canonicalizar índices equivalentes** de forma mais ampla.

## 5. Riscos

- **Junções em T.** Quads gulosos de tamanhos diferentes deixam o vértice de um sobre a aresta de outro.
  - Frequência: 0,46 a 0,54 incidências por quad em esferas e cascas, 0,80 numa nuvem aleatória de 50%, 0 no cubo (ref-medido).
  - Em coordenadas de objeto o vértice está exatamente sobre a reta, sem lacuna geométrica.
  - No raster, as arestas são calculadas a partir de extremos diferentes após a transformada, e o passe principal tem MSAA 4× sem rasterização conservadora (`three.rs:23`, `:1099-1130`). Pode sobrar uma lacuna sub-pixel. Se ela é visível **não verificado**. No tracer, Möller-Trumbore com rejeição estrita não é estanque em arestas exatas (`pathtrace.wgsl:185-200`), e esperam-se raros furos, como em qualquer malha existente.
  - Decisão por medição: a V1 entrega sem costura, e uma sonda de pixels (pixels de fundo dentro da silhueta de um corpo fechado em rotação) decide. Se necessário, conformar inserindo os vértices T custa cerca de +0,5 vértice e +0,5 triângulo por quad (cerca de +12% de vértices, majorante), e muda os dourados.
- **Normais.** Planas, unitárias, 4 vértices não compartilhados. Escala uniforme no `model` mantém a matriz normal válida. Um transform espelhado inverte o enrolamento como qualquer malha.
- **Sombras e vazamento de luz.**
  - O raster desloca a consulta de sombra por `world + normal*0.5`, fixo em unidades de cena (`three.wgsl:232`), com viés constante (`three.rs:799-820`). Nada disso escala com `cellSize`. Células menores que cerca de 0,5 vazam ou flutuam (derivado, não renderizado).
  - O tracer usa deslocamentos absolutos: 1e-3 de continuação, 1e-2 de origem de sombra e 2e-2 de comprimento (`pathtrace.wgsl:730`, `:755`).
  - Mitigação: documentar `cellSize` mínimo (cerca de 1 para raster, 0,1 para tracer), com oráculo de contato em parede de 1 célula no menor valor.
  - Materiais mask e blend projetam sombra opaca no raster (o pipeline de sombra não tem fragment).
  - Faces coplanares dentro de um objeto não coincidem (3.4).
- **Células diagonais.** Duas células que só se tocam por aresta dão geometria fechada com aresta não-múltipla. O furo geométrico é de medida zero, e o risco real é o do `cellSize` pequeno acima.
- **Memória.** 1.240 B de pico e 832 B residentes por quad, mais 768 B por quad de storage do tracer e um pico de host bem maior no empacotamento. O orçamento com aborto corrente recusa cedo (1.9). Reduzir a cópia de CPU de `upload_mesh` é mudança de API (V2).
- **Crater em objeto de voxels.** Um filho `crater` clona e reenvia o array de vértices todo frame (`render_three.rs:1203-1236`). Ver decisão 7.
- **Número de draws.** Grupos × peças, 256 B por draw e por view.

## 6. API que preciso de Occupancy e do importador

**De Occupancy (Netuno)**, em ordem de prioridade:
1. **`lineage() -> u64`** (necessário para a correção). Um id de processo atribuído por contador atômico em `new`/`from_*` e reatribuído por um `Clone` escrito à mão. Nunca entra em nenhum hash de saída, então não mexe em dourados. Garantir que `revision()` é monotônica dentro de uma lineage. Alternativa: um `brick_hash(key)` calculado sob demanda.
2. **`pub use occupancy::Occupancy;`** na raiz do sr-3d, para valer o nome `sr_3d::Occupancy`.
3. **Contrato de compactação:** a revision construída pelo mesher é a cerca passada a `compact(rev)`. Manter a assimetria atual de `bricks()` contra `changed_bricks_since()` (brick esvaziado só na segunda).
4. **`brick(&self, key: [i32;3]) -> Option<&[u8;512]>`**, `None` para brick ausente ou esvaziado, O(log B). Não é bloqueante: sem ele coleto `bricks()` num índice local por remalha. Com ele a remalha incremental lê só as camadas sujas.
5. Conveniência opcional: `bounds()` (mínimo e máximo inclusivos das chaves de célula preenchidas). O mesher deriva isso numa passada.
6. Correção pequena: `appearance_fingerprint()` hasheia as 256 cores, inclusive a entrada 0 (`occupancy.rs:214-224`), que a doc diz não importar. Deveria ignorar a entrada 0.
7. Fora do meu caminho, mas deve-se evitar no caminho por frame: `cells()` (coleta e ordena tudo), `components()` e `fingerprint()`.

**Do importador (Saturno):**
- Um `Occupancy` com a paleta definida por `set_palette` (entrada c−1 do arquivo vai ao índice c). Dizer qual cor vale quando o arquivo não tem paleta (o padrão do Occupancy é branco opaco).
- Por índice 1..255: opaco ou kind (diffuse/metal/glass/emissive), roughness, metallic, transmission, ior, emissão como cor linear × força com unidades documentadas, e alpha. Índices ausentes do grid são ignorados.
- Uma assinatura de conteúdo dessas propriedades, para a chave de cache mudar quando elas mudarem.
- Eixos já em eixos de cena e origem de célula no canto da caixa ocupada (como combinado).

**Do evaluator e do renderer** (meu lado, para a coordenação):
- Grids estáticos: cache `Arc<Occupancy>` no Program (Mutex).
- Grids de simulação (rigidBody `shape="voxels"`, cratera, fragmentos): `FrameNode.voxels: Option<Arc<SimVoxels>>`, publicado como snapshot por revision, uma cópia O(bricks) só quando a revision muda. Fragmentos seguem o padrão de `fracture`: uma malha por fragmento e a pose em `Draw3.model` (`render_three.rs:1721-1790`).

## 7. Decisões que são do coordenador

1. **Semântica de revision.** O código é um contador por linhagem e o enunciado diz hash. Recomendo: contador por linhagem mais `lineage()`, com `fingerprint()` só para identidade entre linhagens e dourados. É a única mudança estrutural que peço ao Netuno.
2. **Convenção de emissão.** E1 (exato, um draw por emissor distinto) ou E2 (um grupo unlit)? Recomendo E1 por padrão e E2 só para `kind` emissor puro, se o Saturno quiser. A flag emissiva em campo de vértice exige shader e fica na V2.
3. **Interface vidro-vidro de índices diferentes.** A V1 emite uma face só (dono = menor classe) e some no raster pelo lado oposto. Alternativa: emitir as duas faces, cada uma visível do seu lado no raster, mas com risco de coincidência no tracer (**não verificado**). Recomendo manter o dono único, documentar e adiar. Aprovar essa limitação para a V1?
4. **Equivalência de classes na V1.** Fundir índices de aparência igual (`class[256]`) dá 6 quads para o cubo sob `@material` único em vez de 53 mil, ao custo de a chave de geometria depender da partição. Recomendo incluir, pois palette > material do objeto > arquivo torna o caso comum.
5. **Peças por faixa canônica (não espaciais) na V1.** O corte de sombra por peça fica inútil e cada edição reenvia todas as peças do grupo afetado. Recomendo aceitar na V1 e deixar as peças espaciais com atualização parcial para a V2.
6. **Padrão de `surfaceMemoryMiB`.** Recomendo 128, igual ao oceano, com mensagem de erro dizendo o quanto falta. A casca de 1 M precisa de 748 e o autor declara explicitamente. Um padrão de 256 admite 216.480 quads e ainda assim não cobre a casca.
7. **`crater` em objeto de voxels.** Recomendo recusar por regra do schema (a deformação de voxels é feita por edição de Occupancy pela física), em vez de aceitar um reenvio do array inteiro por frame.
8. **`cellSize` pequeno.** Recomendo aviso (nota `unsupported`) abaixo de 0,5 no raster e de 0,1 no tracer, e não erro.

## 8. Estimativa de implementação

Onze commits, cada um com o oráculo antes do código:

0. **Identidade pré-mudança** (S). Fixar hashes de shaders, de `node_hash` e de pixels de cenas existentes.
1. **Referência e API de Occupancy** (S). Script de referência em Python no repositório, `lineage`, re-export e contrato de `compact` (com o Netuno). Testes: lineage de clones distinta, `get` não muda lineage.
2. **`voxel::exposure`** (M). Bitboards e regra de exposição. Testes: domino 10 e 11, célula única 6 faces, F do furo, F do tabuleiro, bloco em chaves negativas.
3. **`voxel::greedy`** (M). Fusão global por plano, ordem canônica e tabela de classes. Testes na ordem: cubo = 6, furo = 16 com a ordem dos quatro quads, margem 0 = 10, dourados dos três fixtures, cobertura exata, determinismo por inserção embaralhada.
4. **Expansão para `Vertex`** (S). Testes: layout de 96 B, enrolamento, tangente, AABB, conservação de área, hash dos bytes expandidos.
5. **Orçamento** (S). Aborto corrente, mensagem de erro, constantes 108.240 e companhia, recusa do tabuleiro, grid vazio sem draw.
6. **Cache por fatia e incremental** (M). Incremental = completo por fatia em edições aleatórias, brick esvaziado, retrocesso de revision e troca de lineage.
7. **Schema e eval** (M). `voxels` no enum, `surfaceMemoryMiB`, regras com portão de versão 1.3 e `crater` recusado, `asset_ref`, `timed` do `voxelAsset`, termo de `node_hash` só em `Some`, caso de CLI em `solver_failures.rs`.
8. **Ramo do renderer** (M). Grupos por assinatura, peças, chave de cache, despejo por nó, teto de dispositivo. Testes sem GPU: mesmo `Arc` para mesma revision, despejo na revision nova, recolorir sem remalha. Estatísticas `voxel_mesh_seconds`, `voxel_quads` e `voxel_groups`.
9. **Oráculos de pixel** (S, GPU ou software conforme o caso). Bloco contra cuboide, paleta contra material de documento em espaços sRGB e não-sRGB, contato de sombra em `cellSize` mínimo, sonda de junções em T.
10. **Sonda de custo** (S). `tools/probe_render.py` sobre esfera, casca e nuvem de ruído, para substituir as estimativas por medições antes de prometer números de 1 M de células.
11. **Cache por fatia incremental no renderer** (opcional, depois de 10), só se a sonda mostrar o mesher como gargalo.

Complexidade: S = menos de um dia, M = alguns dias, estimativa grosseira e não medida.

Arquivos desta pesquisa: nenhum arquivo do repositório foi alterado. Scripts de apoio em `/tmp/claude-1000/-home-pals-src-rs-scene-render/8e0c69e6-b97d-44af-abdc-223a64772d14/scratchpad/` (`v3_ref.py`, `greedy_count.py`).