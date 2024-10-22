#! Copyright (c) 2023 University of New Hampshire
#! SPDX-License-Identifier: MIT

def write_doc(outfile):
    outfile.write("\"{}\"".format(name.strip()) + ": {" + doc.format(
        serial.strip(),
        fqdn.strip(),
        host_number.strip(),
        iol_id.strip(),
        ipmi_mac.strip(),
        sp1_mac.strip(),
        ipmi_user.strip(),
        ipmi_pass.strip(),
    ) + "\n	},\n")

def print_doc():
    print("\"{}\"".format(name.strip()) + ": {" + doc.format(
        serial.strip(),
        fqdn.strip(),
        host_number.strip(),
        iol_id.strip(),
        ipmi_mac.strip(),
        sp1_mac.strip(),
        ipmi_user.strip(),
        ipmi_pass.strip(),
    ) + "\n	},\n")

infile = open("temp", "r")
outfile = open("template2", "a")
lines = infile.readlines()
doc = "\n		\"serial\": \"{}\",\n		\"fqdn\": \"{}\",\n		\"host_number\": \"{}\",\n		\"iol_id\": \"{}\",\n		\"ipmi_mac\": \"{}\",\n		\"spl_mac\": \"{}\",\n		\"ipmi_user\": \"{}\",\n		\"ipmi_pass\": \"{}\""
i = 0

name = ""
serial = ""
fqdn = ""
host_number = ""
iol_id = ""
ipmi_mac = ""
sp1_mac = ""
ipmi_user = ""
ipmi_pass = ""

outfile.write("{\n")
while i < len(lines):
    line = lines[i]
    if line[0:3] == "HPE":
        name = line
        i += 1
        serial = lines[i]
        i += 2
        fqdn = lines[i]
        i += 1
        ipmi_user = lines[i]
        i += 1
        ipmi_pass = lines[i]
        i += 1
        host_number = lines[i]
        i += 1
        iol_id = lines[i]
        i += 1
        ipmi_mac = lines[i]
        i += 1
        sp1_mac = lines[i]
        write_doc(outfile)
    else:
        if line[0:3] == "ARM":
            name = line
            i += 2
            fqdn = lines[i]
            i += 1
            ipmi_user = lines[i]
            i += 1
            ipmi_pass = lines[i]
            i += 1
            host_number = lines[i]
            serial = ""
            iol_id = ""
            ipmi_mac = ""
            sp1_mac = ""

            write_doc(outfile)
    i += 1
outfile.write("}\n")
infile.close()
outfile.close()