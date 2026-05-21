{{/* templates/_helpers.tpl */}}

{{/*
The base name is truncated to 51 chars so that the longest suffix we append
("-tls-cluster", 12 chars) still fits inside the DNS-1123 label limit of 63.
Keep this in sync with the longest suffix used in any template.
*/}}
{{- define "stubby.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 51 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s" .Release.Name | trunc 51 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "stubby.labels" -}}
app.kubernetes.io/name: stubby
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
{{- end -}}

{{- define "stubby.selectorLabels" -}}
app.kubernetes.io/name: stubby
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "stubby.image" -}}
{{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}
{{- end -}}
