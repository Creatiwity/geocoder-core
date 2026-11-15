{{/*
Expand the name of the chart.
*/}}
{{- define "app.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Fully qualified app name, truncated at 63 chars (DNS naming spec). A release
name containing the chart name is used as is.
*/}}
{{- define "app.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "app.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "app.labels" -}}
helm.sh/chart: {{ include "app.chart" . }}
{{ include "app.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "app.selectorLabels" -}}
app.kubernetes.io/name: {{ include "app.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "app.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "app.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Countries, validated: fail at render time rather than in a crashing pod
*/}}
{{- define "app.countries" -}}
{{- $known := list "fr" "lu" "be" -}}
{{- $out := list -}}
{{- range splitList "," (.Values.countries | toString) -}}
{{- $c := . | trim | lower -}}
{{- if $c -}}
{{- if not (has $c $known) -}}
{{- fail (printf "countries: unknown country %q (known: fr, lu, be)" $c) -}}
{{- end -}}
{{- $out = append $out $c -}}
{{- end -}}
{{- end -}}
{{- if not $out -}}
{{- fail "countries: at least one of fr, lu, be" -}}
{{- end -}}
{{- with .Values.defaultCountry -}}
{{- if not (has (. | lower) $out) -}}
{{- fail (printf "defaultCountry %q is not in countries" .) -}}
{{- end -}}
{{- end -}}
{{- join "," (uniq $out) -}}
{{- end }}

{{/*
Environment of every container: countries, sources, logs, Sentry, then the
user's extraEnv
*/}}
{{- define "app.env" -}}
{{- $v := .Values -}}
- name: "GEOCODER_COUNTRIES"
  value: {{ include "app.countries" . | quote }}
{{- with $v.defaultCountry }}
- name: "GEOCODER_DEFAULT_COUNTRY"
  value: {{ . | lower | quote }}
{{- end }}
- name: "GEOCODER_DATA_DIR"
  value: {{ $v.persistence.mountPath | quote }}
{{- range $code, $urls := $v.sourceUrls }}
{{- if $urls }}
- name: {{ printf "GEOCODER_SOURCE_URLS_%s" (upper $code) | quote }}
  value: {{ (kindIs "slice" $urls | ternary (join "," $urls) $urls) | quote }}
{{- end }}
{{- end }}
{{- with $v.indexes.baseUrl }}
- name: "GEOCODER_INDEX_BASE_URL"
  value: {{ . | quote }}
{{- end }}
{{- with $v.indexes.publicKeys }}
- name: "GEOCODER_INDEX_PUBLIC_KEYS"
  value: {{ . | quote }}
{{- end }}
- name: "RUST_LOG"
  value: {{ $v.rustLog | quote }}
- name: "GEOCODER_ENV"
  value: {{ $v.environment | quote }}
{{- if or $v.secrets.existingSecret $v.sentryDsn }}
- name: "SENTRY_DSN"
  valueFrom:
    secretKeyRef:
      name: {{ $v.secrets.existingSecret | default (include "app.fullname" .) | quote }}
      key: {{ $v.secrets.keys.sentryDsn | quote }}
      {{- if $v.secrets.existingSecret }}
      optional: true
      {{- end }}
{{- end }}
{{- with $v.extraEnv }}
{{ toYaml . }}
{{- end }}
{{- end }}

{{- define "app.envFrom" -}}
{{- with .Values.envFrom }}
envFrom:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- end }}

{{/*
Labels of the job pods: never the selector labels, or the Service would pick
them up (and the sameNode pod affinity would target them)
*/}}
{{- define "app.jobPodLabels" -}}
app.kubernetes.io/instance: {{ .root.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- with .root.Values.podLabels }}
{{ toYaml . }}
{{- end }}
{{- end }}

{{/*
Pod spec fields shared by every workload
  (dict "root" . "jobs" <bool>)
- jobs: scheduling from `jobs.*`, falling back to the top-level values, and
  next to the API pods when the ReadWriteOnce volume must be shared
  (persistence.sameNode).
*/}}
{{- define "app.podCommon" -}}
{{- $v := .root.Values -}}
{{- $jobs := $v.jobs | default dict -}}
{{- $nodeSelector := $v.nodeSelector -}}
{{- $tolerations := $v.tolerations -}}
{{- $affinity := deepCopy ($v.affinity | default dict) -}}
{{- if .jobs -}}
{{- $nodeSelector = $jobs.nodeSelector | default $nodeSelector -}}
{{- $tolerations = $jobs.tolerations | default $tolerations -}}
{{- $affinity = deepCopy ($jobs.affinity | default $affinity) -}}
{{- end -}}
{{- if $v.persistence.sameNode -}}
{{- $term := dict "labelSelector" (dict "matchLabels" (include "app.selectorLabels" .root | fromYaml)) "topologyKey" "kubernetes.io/hostname" -}}
{{- $podAffinity := get $affinity "podAffinity" | default dict -}}
{{- $required := get $podAffinity "requiredDuringSchedulingIgnoredDuringExecution" | default list -}}
{{- $_ := set $podAffinity "requiredDuringSchedulingIgnoredDuringExecution" (append $required $term) -}}
{{- $_ := set $affinity "podAffinity" $podAffinity -}}
{{- end -}}
automountServiceAccountToken: {{ $v.serviceAccount.automount }}
{{- with $v.imagePullSecrets }}
imagePullSecrets:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with $v.priorityClassName }}
priorityClassName: {{ . }}
{{- end }}
{{- with $v.podSecurityContext }}
securityContext:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with $nodeSelector }}
nodeSelector:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with $tolerations }}
tolerations:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- with $affinity }}
affinity:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- end }}

{{- define "app.dataClaim" -}}
{{- .Values.persistence.existingClaim | default (printf "%s-data" (include "app.fullname" .)) }}
{{- end }}

{{- define "app.dataVolume" -}}
- name: data
  persistentVolumeClaim:
    claimName: {{ include "app.dataClaim" . }}
{{- end }}

{{- define "app.authPolicyName" -}}
{{- printf "%s-auth" (include "app.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}
