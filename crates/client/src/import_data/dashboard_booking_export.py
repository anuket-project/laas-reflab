#! Copyright (c) 2023 University of New Hampshire
#! SPDX-License-Identifier: MIT

from booking.models import *
import json

full = []
for booking in Booking.objects.all():
    output = {}
    purpose = ""
    try:
        purpose = booking.purpose
    except:
        purpose = ""
    project = ""
    try:
        project = booking.project
    except:
        project = ""
    job = ""
    try:
        job = booking.job
    except:
        job = ""
    lab = ""
    try:
        lab = booking.lab
    except:
        lab = ""
    booking_id = ""
    try:
        booking_id = booking.id
    except:
        booking_id = ""
    owner = ""
    try:
        owner = booking.owner.username
    except:
        owner = ""
    pdf = ""
    try:
        pdf = booking.pdf
    except:
        pdf = ""
    output['booking_meta'] = {
        'purpose': purpose,
        'project': project,
        'end': booking.end,
        'start': booking.start,
        'complete': booking.complete,
        'job': job,
        'lab': lab,
        'id': booking_id,
        'owner': owner,
        'collaborators': [user.username for user in booking.collaborators.all()],
        'pdf': pdf,
    }
    output['hosts'] = []
    output['networks'] = {}
    bundle = "No resources"
    try:
        bundle = booking.resource
        servers = bundle.get_resources()
        for server in servers:
            host = {'name': server.name, 'labid': server.labid}
            output['hosts'].append(host)
        physical_networks = bundle.physicalnetwork_set.all()
        for pn in physical_networks:
            data = {
                'public': pn.generic_network.is_public,
                'vlan_id': pn.vlan_id,
            }
            output['networks'][pn.generic_network.name] = data
        full.append(output)
    except:
        output['hosts'] = {}
        output['networks'] = {}

f = open("booking_export.json", 'a')
f.write(json.dumps(full, sort_keys=True, indent=4, default=str))
f.close()
