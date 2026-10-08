# Fase 2 — Acoplamento causal: relatório de fechamento (rascunho)

Código: `phase2/integration` em `5e6ce7d` (código verificado em `fd3c8f0`; acima dele só o ledger). Base da fase: `8af44e9`. Nada foi enviado ao remoto nem fundido em `main`.

## O que a fase entregou

A cadeia causal corre sem nenhum atributo de tempo autoral nas cenas de aceitação:
contato do mundo rígido → cratera pela lei de escala (Holsapple) → ejetos pela lei de Housen & Holsapple (0,8ρV exato) → fumaça e calor por fração da energia → onda do mar pela elevação do fundo e pela cavidade de entrada → respingo dos ejetos na água → empuxo, arrasto de forma e reação horizontal nos corpos → pressão do fundo creditada a cada corpo com balanço exato.

1. Grupo acoplado com registro de trocas e cache de física SRPHYS04; o oceano puxa o emissor de partículas de que precisa; uma física que não responde falha o passo em vez de ser lida como parada.
2. Oceano sobre fundo móvel e corpos (`ocean@colliders`), resposta por profundidade (filtro de Kajiura) ou hidrostática, cavidade de entrada com a profundidade da lei formada ao longo do tempo da lei, respingo esparso.
3. Corpos na água: empuxo, arrasto de forma, `bodyCoupling=none|buoyancy|full`.
4. Contato como fonte: cratera, ejetos, fumaça, `crater@capture`, `physics@fixInternalEdges`, aviso W02.
5. Partículas: arrasto pelo gás da fumaça, nascimento na superfície que a cratera tem na hora, atrito 0,7 e raio físico declarados.
6. Render: grade de luz por fonte para meios (UHD 121–355 s → 8,8 s na hero), luz pela água (absorção, sombra refratada, domo atrás do vidro), referências de força bruta dimensionadas ao adaptador.

## Números que sustentam o fecho (todos no SREP com commit, data e carga)

- Balanço de momento da água na cena de empuxo: resíduo 0,00% hidrostático, 0,01–0,09% filtrado (paredes); 3–15% antes do crédito exato.
- Ondas do mar (3 m, só ordem afirmada): campo distante por velocidade 0,91/1,51/2,30 m, por massa 1,14/1,51/2,71 m; cratera sozinha 0,006/0,012/0,021 m filtrado contra 0,16/0,27/0,40 m hidrostático.
- Ejetos: 0 de 4000 atravessam o chão com raio 0,17 m (eram 754); rocha de 270 t com atrito não para o solver.
- Custo: oceano acoplado dentro do ruído (0,053–0,065 s/quadro na cena autoral); respingo invisível; crédito exato 0,92–1,01 do custo anterior.
- Determinismo: bit a bit entre 1/2/8 threads e em replay por checkpoint em todos os solvers tocados; sonda da hero 720p com o hash da Fase 1.

## Limites declarados (SREP e ledger)

- Ejetos com raio abaixo da flecha das facetas da malha (1 m) ainda atravessam (~1500 de 4000 a 0,02 m).
- Um passo rígido em contato com a cratera que deforma custa 24–33 ms (item de desempenho da Fase 3).
- A cavidade é deslocamento hidrostático sem jato nem coroa; variação de leito sem dono (cratera) não é creditada.
- O plano de mar distante da cena do oceano é transmissivo e esconde o leito (Fase 4).
- Dois testes do adaptador de software são instáveis sob carga nos dois commits medidos (qualidade de imagem, não tempo).

## Processo

108 commits, 107 integrados em 26 merges, 11 verificações completas. Três defeitos reais achados por testes novos (dois de agendamento, um de determinismo) e corrigidos. Hipóteses do coordenador refutadas por medição: 3 no dia do fecho; registradas.

## Pacote de sign-off

/home/pals/renders/cinematic-impact/phase2-close/ — a preencher com os caminhos e hashes do MANIFEST.txt.

## Próximo passo

Fase 3 — Física que falta: fratura por impulso de contato, pressão da fumaça sobre corpos, rastro dos ejetos, colisor da cratera barato, escala de tempo uniforme no grupo acoplado.
