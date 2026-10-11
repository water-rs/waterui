{{ begin }}
{%- for plugin in plugins %}
        id("{{ plugin.id }}") version "{{ plugin.version }}"
{%- endfor %}
{{ end }}
