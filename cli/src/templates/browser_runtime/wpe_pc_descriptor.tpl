prefix=${pcfiledir}/{{ parents }}
{{ segments[0] }}{% for segment in tail %}${prefix}{{ segment }}{% endfor %}