#! Copyright (c) 2023 University of New Hampshire
#! SPDX-License-Identifier: MIT

for net in PublicNetwork.objects.all():
    print(net.vlan)