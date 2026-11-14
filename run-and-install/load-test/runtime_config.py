"""Read the private Jenkins/Vault configuration without dotenv interpolation."""
import json
from urllib.parse import urlsplit


def database_settings(values):
    """Normalize native DB_* and shared SCHOOL_BUS_DATASOURCE_* settings."""
    result = {key: value for key, value in values.items() if key.startswith('DB_')}
    if not result.get('DB_HOST') and values.get('SCHOOL_BUS_DATASOURCE_URL'):
        try:
            raw = values['SCHOOL_BUS_DATASOURCE_URL']
            url = urlsplit(raw[5:] if raw.startswith('jdbc:') else raw)
            if url.scheme != 'mysql' or not url.hostname or not url.path.strip('/') or url.username or url.password:
                raise ValueError()
            result.update(DB_HOST=url.hostname, DB_PORT=str(url.port or 3306), DB_NAME=url.path.lstrip('/'))
        except (ValueError, TypeError):
            raise ValueError('SCHOOL_BUS_DATASOURCE_URL must identify a MySQL database') from None
    for native, shared in (('DB_USER', 'SCHOOL_BUS_DATASOURCE_USERNAME'),
                           ('DB_PASSWORD', 'SCHOOL_BUS_DATASOURCE_PASSWORD')):
        if not result.get(native) and shared in values:
            result[native] = values[shared]
    return result


def load_runtime_config(path):
    try:
        values = json.loads(path.read_text())
        keys = ('SCHOOL_BUS_DATASOURCE_URL', 'SCHOOL_BUS_DATASOURCE_USERNAME',
                'SCHOOL_BUS_DATASOURCE_PASSWORD', 'REAXIUM_API_USER', 'REAXIUM_API_PASSWORD')
        if not isinstance(values, dict) or any(not isinstance(values.get(key), str) or not values[key] for key in keys):
            raise ValueError()
        # Only use the explicitly provisioned shared keys, never ambient DB_*.
        result = database_settings({key: values[key] for key in keys})
        result.update(REAXIUM_API_USER=values['REAXIUM_API_USER'],
                      REAXIUM_API_PASSWORD=values['REAXIUM_API_PASSWORD'])
        return result
    except (ValueError, TypeError):
        raise ValueError('Invalid Vault runner configuration; require SCHOOL_BUS_DATASOURCE_URL, '
                         'SCHOOL_BUS_DATASOURCE_USERNAME, SCHOOL_BUS_DATASOURCE_PASSWORD, '
                         'REAXIUM_API_USER and REAXIUM_API_PASSWORD') from None
