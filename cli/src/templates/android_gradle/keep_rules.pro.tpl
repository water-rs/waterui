{{ begin }}
{%- for (class, members) in classes %}
{%- if members.is_empty() %}
-keep class {{ class }}
{%- else %}
-keep class {{ class }} {
{%- for member in members %}
    {{ member }};
{%- endfor %}
}
{%- endif %}
{%- endfor %}
{{ end }}
