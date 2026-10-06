{%- macro common(component) -%}
{%- if let Some(enabled) = component.enabled %}
            android:enabled="{{ enabled }}"
{%- endif %}
{%- if let Some(permission) = component.permission %}
            android:permission="{{ permission }}"
{%- endif %}
{%- if let Some(process) = component.process %}
            android:process="{{ process }}"
{%- endif %}
{%- if let Some(direct_boot_aware) = component.direct_boot_aware %}
            android:directBootAware="{{ direct_boot_aware }}"
{%- endif %}
{%- endmacro -%}

{%- macro intent_filters(filters) -%}
{%- for filter in filters.iter() %}
            <intent-filter
{%- if let Some(priority) = filter.priority %} android:priority="{{ priority }}"{% endif %}>
{%- for action in filter.actions %}
                <action android:name="{{ action }}" />
{%- endfor %}
{%- for category in filter.categories %}
                <category android:name="{{ category }}" />
{%- endfor %}
{%- for data in filter.data %}
                <data
{%- if let Some(scheme) = data.scheme %} android:scheme="{{ scheme }}"{% endif %}
{%- if let Some(host) = data.host %} android:host="{{ host }}"{% endif %}
{%- if let Some(port) = data.port %} android:port="{{ port }}"{% endif %}
{%- if let Some(path) = data.path %} android:path="{{ path }}"{% endif %}
{%- if let Some(path_prefix) = data.path_prefix %} android:pathPrefix="{{ path_prefix }}"{% endif %}
{%- if let Some(path_pattern) = data.path_pattern %} android:pathPattern="{{ path_pattern }}"{% endif %}
{%- if let Some(mime_type) = data.mime_type %} android:mimeType="{{ mime_type }}"{% endif %} />
{%- endfor %}
            </intent-filter>
{%- endfor %}
{%- endmacro -%}

{%- macro component_meta_data(entries) -%}
{%- for entry in entries.iter() %}
            <meta-data android:name="{{ entry.name }}" android:{{ entry.attribute() }}="{{ entry.content() }}" />
{%- endfor %}
{%- endmacro -%}

{{ begin|safe }}
{%- for provider in providers %}
        <provider
            android:name="{{ provider.name }}"
            android:authorities="{{ provider.authorities_attribute() }}"
            android:exported="{{ provider.exported }}"
{%- call common(provider) %}{%- endcall %}
{%- if let Some(grant_uri_permissions) = provider.grant_uri_permissions %}
            android:grantUriPermissions="{{ grant_uri_permissions }}"
{%- endif %}
{%- if let Some(read_permission) = provider.read_permission %}
            android:readPermission="{{ read_permission }}"
{%- endif %}
{%- if let Some(write_permission) = provider.write_permission %}
            android:writePermission="{{ write_permission }}"
{%- endif %}
{%- if let Some(multiprocess) = provider.multiprocess %}
            android:multiprocess="{{ multiprocess }}"
{%- endif %}
{%- if let Some(init_order) = provider.init_order %}
            android:initOrder="{{ init_order }}"
{%- endif %}
{%- if let Some(syncable) = provider.syncable %}
            android:syncable="{{ syncable }}"
{%- endif %}
{%- if provider.meta_data.is_empty() %} />
{%- else %}>
{%- call component_meta_data(provider.meta_data) %}{%- endcall %}
        </provider>
{%- endif %}
{%- endfor %}
{%- for service in services %}
        <service
            android:name="{{ service.name }}"
            android:exported="{{ service.exported }}"
{%- call common(service) %}{%- endcall %}
{%- if let Some(foreground_service_type) = service.foreground_service_type_attribute() %}
            android:foregroundServiceType="{{ foreground_service_type }}"
{%- endif %}
{%- if let Some(isolated_process) = service.isolated_process %}
            android:isolatedProcess="{{ isolated_process }}"
{%- endif %}
{%- if let Some(stop_with_task) = service.stop_with_task %}
            android:stopWithTask="{{ stop_with_task }}"
{%- endif %}
{%- if service.intent_filter.is_empty() && service.meta_data.is_empty() %} />
{%- else %}>
{%- call intent_filters(service.intent_filter) %}{%- endcall %}
{%- call component_meta_data(service.meta_data) %}{%- endcall %}
        </service>
{%- endif %}
{%- endfor %}
{%- for receiver in receivers %}
        <receiver
            android:name="{{ receiver.name }}"
            android:exported="{{ receiver.exported }}"
{%- call common(receiver) %}{%- endcall %}
{%- if receiver.intent_filter.is_empty() && receiver.meta_data.is_empty() %} />
{%- else %}>
{%- call intent_filters(receiver.intent_filter) %}{%- endcall %}
{%- call component_meta_data(receiver.meta_data) %}{%- endcall %}
        </receiver>
{%- endif %}
{%- endfor %}
{%- for entry in meta_data %}
        <meta-data android:name="{{ entry.name }}" android:{{ entry.attribute() }}="{{ entry.content() }}" />
{%- endfor %}
        {{ end|safe }}
