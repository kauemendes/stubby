# stubby — Design Spec

**Date:** 2026-05-20
**Status:** Approved (brainstorm phase)
**Author:** Kauê Mendes (with Claude)

## 1. Goal

`stubby` é um Mutating Admission Webhook para Kubernetes que substitui automaticamente a `image` de containers em Pods por imagens *dummy* (backend HTTP ou frontend HTML) quando o Pod (ou seu PodTemplate) carrega uma annotation declarando o tipo.

O objetivo do projeto é duplo:

1. **Aprendizado:** servir como laboratório prático de desenvolvimento de aplicações para clusters Kubernetes (admission webhooks, JSONPatch, RBAC, TLS bootstrap, Helm, kube-rs).
2. **Utilidade real:** permitir que times levantem rapidamente um esqueleto de serviço em qualquer cluster antes da imagem real estar pronta, instalando o sistema via Helm de um registry público (GHCR).

## 2. Non-Goals

Itens explicitamente fora de escopo da v1 — qualquer um deles é roadmap, não requisito:

- Detecção automática de "imagem real ficou pronta" (exigiria controller observando `ImagePullBackOff`).
- CRDs próprios — annotations cobrem a UX desejada.
- Operator/reconcile loop.
- Dummies de tipo Worker / CronJob / gRPC (roadmap, não v1).
- Suporte a múltiplos runtimes além de Kubernetes padrão (`MutatingWebhookConfiguration` v1).
- Mecanismo de "fallback se a imagem real falhar" — usuário troca annotation para reverter.

## 3. User Experience

### 3.1 Annotation API

Annotations declaradas em `spec.template.metadata.annotations` do Deployment (ou diretamente em `metadata.annotations` do Pod):

| Annotation | Valores | Default | Descrição |
|---|---|---|---|
| `stubby.io/type` | `backend` \| `frontend` \| `off` | ausente = não injeta | Tipo de dummy. `off` desliga sem precisar remover a annotation. |
| `stubby.io/app-name` | string livre | `metadata.name` | Nome exibido pelo dummy (header HTTP, título HTML). |
| `stubby.io/port` | u16 (1–65535) | backend `8080`, frontend `80` | Porta que o dummy escuta dentro do container. |
| `stubby.io/image-override` | string (`registry/imagem:tag`) | imagem oficial GHCR | Permite usar imagem própria do usuário em vez da publicada pelo projeto. |

### 3.2 Fluxo do usuário

1. Dev aplica um `Deployment` com `stubby.io/type: backend` no `podTemplate`.
2. API server cria os Pods → chama o webhook do `stubby`.
3. Webhook devolve um JSONPatch que troca `image`, `ports`, `livenessProbe`, `readinessProbe`, `command`, `args` por valores compatíveis com o dummy.
4. Pod sobe imediatamente com a imagem dummy; `/health` responde 200.
5. Quando a imagem real estiver pronta, o dev troca `stubby.io/type` para `off` (ou remove a annotation) e reaplica o manifesto.

### 3.3 Exemplo mínimo

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: orders-api
spec:
  replicas: 1
  selector: { matchLabels: { app: orders-api } }
  template:
    metadata:
      labels: { app: orders-api }
      annotations:
        stubby.io/type: backend
        stubby.io/app-name: "Orders API"
    spec:
      containers:
        - name: orders
          image: ghcr.io/exemplo/orders-api:latest  # ignorada pelo webhook enquanto annotation existir
```

## 4. Architecture

### 4.1 Componentes

```
                            ┌──────────────────────────────┐
                            │   Kubernetes API server      │
                            └──────────────┬───────────────┘
                                           │ AdmissionReview
                                           ▼
                            ┌──────────────────────────────┐
                            │  stubby-webhook (Rust/axum)  │  ← crates/webhook
                            │  - parse AdmissionReview     │
                            │  - build JSONPatch           │
                            │  - return AdmissionReview    │
                            └──────────────┬───────────────┘
                                           │ JSONPatch
                                           ▼
                            ┌──────────────────────────────┐
                            │   Pod final (mutated)        │
                            │   image: stubby-dummy-*      │
                            └──────────────────────────────┘

Imagens publicadas no GHCR:
  ghcr.io/<org>/stubby-webhook:<ver>         ← crates/webhook
  ghcr.io/<org>/stubby-dummy-backend:<ver>   ← crates/dummy-backend
  ghcr.io/<org>/stubby-dummy-frontend:<ver>  ← crates/dummy-frontend
```

- **`crates/webhook`** — servidor HTTPS axum + kube-rs que implementa `AdmissionReview/v1`. Sem estado, escalável horizontalmente.
- **`crates/dummy-backend`** — axum minimal. Endpoints: `/health`, `/ready`, `/metrics`, `/openapi.json`, `/docs` (swagger-ui estático embedado via `include_dir!`), catch-all `*` → `{"status":"dummy","app":"<nome>","path":"<url>"}` com `Content-Type: application/json`. Nome do app vem da env `STUBBY_APP_NAME`.
- **`crates/dummy-frontend`** — assets HTML/CSS/JS gerados em `build.rs` a partir de um template; imagem final é `nginx:alpine` com os assets + um `entrypoint.sh` que faz `envsubst` em `index.html` para injetar `STUBBY_APP_NAME`.
- **`charts/stubby`** — Helm chart.

### 4.2 Escopo do webhook

- `MutatingWebhookConfiguration`:
  - `rules`: operações `CREATE` em `pods` (apiGroup `""`, version `v1`).
  - `failurePolicy: Ignore` (webhook fora do ar não trava criação de pods).
  - `sideEffects: None`.
  - `admissionReviewVersions: ["v1"]`.
  - `namespaceSelector`: por padrão exclui `kube-system` e `stubby-system` (label `stubby.io/exclude: "true"` adicionada via chart hook nesses namespaces).
  - `reinvocationPolicy: Never` (não precisa reinvocar).
- Webhook só age sobre **Pods**, nunca sobre Deployments — Deployments propagam annotations para Pods via `podTemplate.metadata`, então o lugar canônico para mutar é o Pod.
- Webhook ignora pods sem a annotation `stubby.io/type` ou com valor `off` / inválido (devolve `AdmissionReview` `allowed: true` sem patch).

### 4.3 Geração do JSONPatch

As annotations são lidas no **nível do Pod** (`metadata.annotations`). Quando `stubby.io/type` está presente e é válido, o patch é aplicado a **todos os containers** em `spec.containers`, exceto sidecars conhecidos e os listados em `stubby.io/skip-containers`. Para cada container alvo, o patch substitui:

- `image` → imagem dummy correspondente (ou `stubby.io/image-override` se presente).
- `ports` → `[{containerPort: <port>, name: "http", protocol: "TCP"}]`.
- `livenessProbe` → `httpGet: {path: "/health", port: <port>}`, `initialDelaySeconds: 1`, `periodSeconds: 10`.
- `readinessProbe` → `httpGet: {path: "/ready", port: <port>}`, `initialDelaySeconds: 1`, `periodSeconds: 5`.
- `command` e `args` → removidos (`{"op": "remove", ...}` quando presentes).
- `env` → adiciona `STUBBY_APP_NAME` (sem remover envs existentes).
- `resources` → mantido se já configurado; senão aplica defaults baixos (`requests: {cpu: 10m, memory: 32Mi}`, `limits: {cpu: 100m, memory: 64Mi}`).

Containers com nomes prefixados por `istio-`, `linkerd-`, `vault-` ou listados em `stubby.io/skip-containers` (CSV) são pulados — evita conflito com sidecars conhecidos.

### 4.4 TLS

O chart oferece dois modos selecionáveis por `values.tls.mode`:

- **`cert-manager`** — gera `Certificate` + `Issuer` (auto-signed); assume cert-manager instalado no cluster.
- **`self-signed`** (default) — `helm install` cria um Job pré-install (`helm.sh/hook: pre-install,pre-upgrade`) que:
  1. Verifica se já existe `Secret` válido com cert não expirado; se sim, encerra (idempotente).
  2. Senão, gera CA + serving cert via `openssl` (imagem `alpine/openssl`), validade 1 ano.
  3. Cria/atualiza `Secret` com `tls.crt` / `tls.key` no namespace do release.
  4. Patcha o `caBundle` do `MutatingWebhookConfiguration` via ServiceAccount com RBAC mínimo (`get`/`patch` em `mutatingwebhookconfigurations` no nome específico).
  5. Em `helm upgrade`, o mesmo Job roda — se o cert estiver a < 30 dias de expirar, rotaciona.

### 4.5 Observabilidade

- Logs JSON estruturados via `tracing` + `tracing-subscriber` (campos: `request_id`, `namespace`, `pod_name`, `annotation_type`, `decision`, `latency_ms`).
- Endpoint `/metrics` (Prometheus) expondo:
  - `stubby_admissions_total{type, decision}` (counter).
  - `stubby_admission_latency_seconds` (histogram).
  - `stubby_patch_errors_total{kind}` (counter).
- Endpoints `/healthz` e `/readyz` para liveness/readiness do próprio webhook.

## 5. Repository Layout

```
stubby/
├── Cargo.toml                       # workspace
├── crates/
│   ├── webhook/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── server.rs            # axum + TLS
│   │   │   ├── admission.rs         # AdmissionReview parsing
│   │   │   ├── patch.rs             # JSONPatch generation (núcleo testável)
│   │   │   └── config.rs            # env, image refs
│   │   └── tests/                   # integration tests (mock AdmissionReview)
│   ├── dummy-backend/
│   │   ├── Cargo.toml
│   │   ├── src/main.rs
│   │   └── tests/
│   └── dummy-frontend/
│       ├── Cargo.toml
│       ├── build.rs                 # gera assets a partir de templates
│       ├── templates/
│       │   └── index.html.tmpl
│       └── nginx/
│           ├── Dockerfile
│           └── entrypoint.sh
├── charts/stubby/
│   ├── Chart.yaml
│   ├── values.yaml
│   ├── templates/
│   │   ├── deployment.yaml
│   │   ├── service.yaml
│   │   ├── mutatingwebhookconfiguration.yaml
│   │   ├── rbac.yaml
│   │   ├── tls-cert-manager.yaml
│   │   ├── tls-self-signed-job.yaml
│   │   └── _helpers.tpl
│   └── tests/                       # helm unittest fixtures
├── examples/
│   ├── backend.yaml
│   └── frontend.yaml
├── docs/
│   ├── README.md
│   ├── installation.md
│   ├── annotations.md
│   └── superpowers/specs/
└── .github/workflows/
    ├── ci.yaml                      # PR gates
    └── release.yaml                 # tag → build, push, sign, publish chart
```

## 6. Testing — TDD by default

TDD é regra padrão e inegociável no projeto: nada de código sem teste falhando antes. Ciclo red → green → refactor em todos os crates e no chart.

### 6.1 Workflow por feature ou bugfix

1. Escrever o teste que captura o comportamento desejado e vê-lo falhar (`cargo test` vermelho).
2. Implementar o mínimo necessário para passar (`cargo test` verde).
3. Refatorar com a rede de segurança dos testes.
4. Commits idealmente separam "red" e "green" para deixar o ciclo visível no histórico.

### 6.2 Camadas

- **Unit `crates/webhook`** — `patch.rs` tem tabela de casos `(AdmissionReview input → JSONPatch esperado)` cobrindo:
  - `type=backend`, `type=frontend`, `type=off`, annotation ausente, annotation inválida.
  - Multi-container pod (um marcado, um sidecar).
  - `image-override`, `port`, `app-name` overrides.
  - Container com sidecar conhecido (istio) — não muta.
  - Pod com `livenessProbe`/`readinessProbe` pré-existentes — substituição.
  - Pod sem `resources` — defaults aplicados; com `resources` — preservados.
- **Unit `crates/dummy-backend`** — handlers axum testados via `tower::ServiceExt::oneshot` (sem subir socket). Cada endpoint (`/health`, `/ready`, `/metrics`, `/openapi.json`, `/docs`, catch-all) tem caso de teste com asserts de status, headers e body shape.
- **Unit `crates/dummy-frontend`** — snapshots `insta` da geração de `index.html` para casos: nome simples, nome com caracteres especiais (XSS guard), nome vazio (usa fallback).
- **Integration** — cluster `kind` provisionado em CI, `helm install ./charts/stubby --set image.repository=local/...`, `kubectl apply` de fixtures em `examples/`, asserts via `kubectl get pod -o jsonpath` (validar `image` mutada) e probes HTTP via `kubectl run curl --rm -i --image=curlimages/curl --restart=Never -- curl -sf http://<svc>:<port>/health` (não dependemos de tooling dentro do container dummy, que é distroless).
- **Chart** — `helm unittest` cobrindo:
  - Defaults sãos.
  - `tls.mode=cert-manager` gera `Certificate`, não gera Job.
  - `tls.mode=self-signed` gera Job + RBAC, não gera `Certificate`.
  - `namespaceSelector` custom propagado corretamente.
- **Smoke pós-deploy** (parte do integration) — dummy-backend responde 200 em `/health` e `/ready`; dummy-frontend serve HTML contendo o `app-name` configurado.

### 6.3 CI gates (PR não merga sem)

- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `helm lint charts/stubby`
- `helm unittest charts/stubby`
- Integração em `kind` em matriz de versões k8s: `v1.29`, `v1.30`, `v1.31`.
- Cobertura via `cargo-llvm-cov`, meta inicial **80%** no `crates/webhook` (núcleo de patch). Cobertura é reportada, mas só `crates/webhook` é gate de bloqueio.

## 7. CI/CD (GitHub Actions)

- **`ci.yaml`** — disparado em PR e push para `main`. Roda todos os gates da seção 6.3.
- **`release.yaml`** — disparado em tags `v*.*.*`:
  1. Build multi-arch (`linux/amd64`, `linux/arm64`) das 3 imagens via `docker buildx`.
  2. Push para `ghcr.io/<org>/stubby-{webhook,dummy-backend,dummy-frontend}:<tag>` + `:latest`.
  3. Sign imagens com `cosign` (keyless via GitHub OIDC).
  4. `helm package charts/stubby` + publicar no branch `gh-pages` como Helm repo.
  5. Criar GitHub Release com notas geradas via `git-cliff` (changelog do tipo conventional commits).

## 8. Open questions (resolver durante o plano)

Itens menores que viraram premissas razoáveis e que podem ser confirmados no plano de implementação:

- Versão mínima de k8s suportada: ficar em `v1.29+` (cobre a maioria dos clusters em produção em 2026).
- Estratégia de bump de versão do chart: `appVersion` ligado à tag do binário; `version` (chart) bump independente.
- Suporte a `ImagePullSecrets` para imagem dummy — não necessário se hospedada em GHCR público; documentar para overrides privados.
- `<org>` referenciada nas imagens (`ghcr.io/<org>/...`) será definida quando o repo for criado no GitHub (provavelmente `kauemendes/stubby`); valores em `values.yaml` e workflows usarão `${{ github.repository_owner }}`.

## 9. Success criteria

A v1 do `stubby` é considerada pronta quando:

1. `helm repo add stubby https://<org>.github.io/stubby && helm install stubby/stubby` instala em um cluster `kind` limpo sem erros.
2. Aplicar `examples/backend.yaml` resulta em pod `Running` em < 10s, com `/health` respondendo 200.
3. Aplicar `examples/frontend.yaml` resulta em pod `Running` em < 10s, com HTML mostrando o `app-name`.
4. Trocar a annotation para `off` e reaplicar resulta em pod tentando puxar a imagem real (comportamento "padrão" do k8s).
5. Cobertura `crates/webhook` ≥ 80%, todos os CI gates verdes.
6. README documenta instalação, annotations, troubleshooting básico.

## 10. Next steps

Após aprovação deste spec:

1. Invocar `superpowers:writing-plans` para gerar plano de implementação detalhado, tarefa por tarefa, **respeitando TDD** (cada tarefa começa pelo teste falhando).
2. Inicializar repositório git, scaffold do workspace Cargo.
3. Implementar na ordem: `crates/webhook` (núcleo) → `crates/dummy-backend` → `crates/dummy-frontend` → `charts/stubby` → CI/CD → docs.
