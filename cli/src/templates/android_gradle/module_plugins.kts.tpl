{{ begin }}
{%- for plugin in plugins %}
    id("{{ plugin.id }}")
{%- endfor %}
{{ end }}
