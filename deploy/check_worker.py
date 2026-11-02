"""Installer-only OpenAI-compatible connection check; never prints credentials."""
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request


def settings(path):
    try:
        with open(path, encoding='utf-8-sig') as stream:
            saved = json.load(stream)
        if not isinstance(saved, dict):
            raise ValueError('config is not an object')
    except FileNotFoundError:
        saved = {}
    except (ValueError, OSError):
        raise ValueError('Cannot read worker settings. Correct the JSON configuration first.') from None
    return {
        name: os.environ.get('MMHMEM_CHECK_' + suffix) or saved.get(name)
        or os.environ.get('WORKER_' + suffix) or default
        for name, suffix, default in (
            ('worker_model', 'MODEL', ''),
            ('worker_api_url', 'API_URL', 'http://127.0.0.1:11434/v1/chat/completions'),
            ('worker_api_key', 'API_KEY', ''),
        )
    }


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def check(config):
    if not config['worker_model']:
        return
    url = urllib.parse.urlsplit(config['worker_api_url'])
    if url.scheme not in ('http', 'https') or not url.hostname or url.username or url.password:
        raise ValueError('Invalid worker URL. Use an HTTP(S) base URL without embedded credentials.')
    print("\n  Testing worker connection with a short 'Reply OK' request.")
    print('  No project files are sent. Provider usage charges may apply.')
    headers = {'Content-Type': 'application/json'}
    if config['worker_api_key']:
        headers['Authorization'] = 'Bearer ' + config['worker_api_key']
    body = json.dumps({'model': config['worker_model'], 'temperature': 0.0, 'messages': [
        {'role': 'system', 'content': 'You are a connection test.'},
        {'role': 'user', 'content': 'Reply OK.'},
    ]}).encode()
    request = urllib.request.Request(config['worker_api_url'], data=body, headers=headers)
    try:
        with urllib.request.build_opener(NoRedirect).open(request, timeout=30) as response:
            result = json.load(response)
    except urllib.error.HTTPError as error:
        reason = {
            401: 'Authentication failed: the API key is invalid or expired.',
            403: 'Access denied: check the API key and model permissions.',
            404: 'Endpoint or model not found: check the base URL and model ID.',
            400: 'Request rejected: check the model ID and OpenAI compatibility.',
            429: 'Provider quota or rate limit reached. Check billing or retry later.',
        }.get(error.code, f'Provider returned HTTP {error.code}. Check the endpoint and retry.')
        raise ValueError(reason) from None
    except (urllib.error.URLError, TimeoutError, OSError):
        raise ValueError('Connection failed or timed out. Check the URL, network, and TLS certificate.') from None
    except ValueError:
        raise ValueError('Worker returned invalid JSON; check the endpoint.') from None
    try:
        content = result['choices'][0]['message']['content']
        if not isinstance(content, str) or not content.strip():
            raise ValueError()
    except (KeyError, IndexError, TypeError, ValueError):
        raise ValueError('Worker returned an invalid chat-completions response.') from None
    print('  Worker connection verified.')


if __name__ == '__main__':
    try:
        config = settings(sys.argv[2])
        if sys.argv[1] == 'show':
            if config['worker_model']:
                url = urllib.parse.urlsplit(config['worker_api_url'])
                # Do not display URL userinfo/query parameters that may contain credentials.
                base = urllib.parse.urlunsplit((url.scheme, url.hostname or '', url.path, '', ''))
                if url.port:
                    base = urllib.parse.urlunsplit((url.scheme, f'{url.hostname}:{url.port}', url.path, '', ''))
                print('  Current base URL: ' + base.removesuffix('/chat/completions'))
                print('  Current model ID: ' + config['worker_model'])
                print('  API key: ' + ('***' if config['worker_api_key'] else '(not set)'))
        elif sys.argv[1] == 'model':
            print(config['worker_model'])
        else:
            check(config)
    except (ValueError, TypeError):
        # Known check errors contain only our fixed messages, never server bodies.
        message = sys.exc_info()[1]
        print(f'Worker check failed:\n{message}\nNo new settings were saved.\nRe-run to correct them.', file=sys.stderr)
        sys.exit(1)
