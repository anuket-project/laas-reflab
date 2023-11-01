#! Copyright (c) 2023 University of New Hampshire
#! SPDX-License-Identifier: MIT

import requests

def endpoint_to_url(endpoint):
    return "http://0.0.0.0:5001" + endpoint

# returns a tuple of (status_code, response_body)
def do(method, endpoint, body=None):
    url = endpoint_to_url(endpoint)
    response = None
    if body:
        response = method(url, json = body)
    else:
        response = method(url)

    response_body = None
    try:
        response_body = response.json()
    except Exception as e:
        print("endpoint did not give back json")

    return (response.status_code, response_body)

def post(endpoint, body=None):
    return do(requests.post, endpoint, body)

def get(endpoint, body=None):
    return do(requests.get, endpoint, body)

class template(object):
    def prefix():
        return "/template"

    class blobs(object):
        def get_list():
            return get(template.prefix() + "/list")

        # returns id of the newly committed template
        def commit(id: str):
            pass

        # returns uuid for a basic blob with one host of an arbitrary flavor (if flavor=None),
        # that has not been committed yet
        def basic_blob(flavor=None, hostname="host"):
            pass


    # returns a uuid for a basic template with one host of an arbitrary flavor
    def basic_template():
        blob_id = blobs.basic_blob()
        template_id = blobs.commit(blob_id)

        return template_id

class booking(object):
    def prefix():
        return "/booking"

    def booking_with(template_id, for_user=42, credentials=[], cifile=""):
        pass

class tests(object):
    def basic_e2e():
        tid = template.basic_template()
        bid = booking.booking_with(tid)

def run_tests():
    tests.basic_e2e()

run_tests()
