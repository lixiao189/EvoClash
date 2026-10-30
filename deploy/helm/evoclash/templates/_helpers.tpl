{{- define "evoclash.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- /* Labels that never take part in selectors; pod templates add the selector labels. */}}
{{- define "evoclash.commonLabels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "evoclash.labels" -}}
{{ include "evoclash.commonLabels" . }}
app.kubernetes.io/name: {{ .Chart.Name }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "evoclash.selectorLabels" -}}
app.kubernetes.io/name: {{ .root.Chart.Name }}
app.kubernetes.io/instance: {{ .root.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{- define "evoclash.serviceAccount" -}}{{ include "evoclash.matchmaker" . }}{{- end -}}

{{- define "evoclash.matchmaker" -}}{{ include "evoclash.fullname" . }}-matchmaker{{- end -}}
{{- define "evoclash.heartbeat" -}}{{ include "evoclash.fullname" . }}-matchmaker-heartbeat{{- end -}}
{{- /* Room for "-<ordinal>" under the 63-character DNS-label limit of a pod hostname. */ -}}
{{- define "evoclash.server" -}}{{ printf "%s-server" (include "evoclash.fullname" . | trunc 52 | trimSuffix "-") }}{{- end -}}

{{- define "evoclash.image" -}}
{{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}
{{- end -}}

{{- define "evoclash.secretName" -}}
{{ .Values.netcodeKey.existingSecret | default (printf "%s-netcode" (include "evoclash.fullname" .)) }}
{{- end -}}

{{- define "evoclash.keyEnv" -}}
- name: EVOCLASH_NETCODE_KEY
  valueFrom:
    secretKeyRef:
      name: {{ include "evoclash.secretName" . }}
      key: {{ .Values.netcodeKey.secretKey }}
{{- end -}}
