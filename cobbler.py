#! Copyright (c) 2023 University of New Hampshire
#! SPDX-License-Identifier: MIT

############################################
#               DEPRECATED
############################################


from xmlrpc.client import ServerProxy
import json

def new_action(config):
    return CobblerAction(config)

class CobblerAction():
    def __init__(self, config=None):
        #super(CobblerAction, self).__init__(config=config)
        self.server_url = config['url']
        self.user = config['user']
        self.password = config['password']
        self.cobbler = ServerProxy(self.server_url)
        print('Attempting login, user:', self.user, 'and password:', self.password)
        self.token = self.cobbler.login(self.user, self.password)
        self._config = config


    def profile_names(self):
        return [ profile['name'] for profile in self.cobbler.get_profiles() ]


    def system_names(self):
        return [ system['name'] for system in self.cobbler.get_systems() ]


    def distro_names(self):
        return [ distro['name'] for distro in self.cobbler.get_distros() ]


    def purge_systems(self):
        for name in self.system_names():
            self.cobbler.remove_system(name, self.token)
        

    def get_default_profile_name(self, arch: str):
        for profile in self.cobbler.get_profiles():
            if self.get_distro(profile['distro'])['arch'] == arch:
                return profile['name']
        return 'no-profile'


    def system_exists(self, hostname: str):
        return hostname in self.system_names()


    def profile_exists(self, profile_name: str):
        return profile_name in self.profile_names()

    
    # returns a system object, modifications to this object are not saved, this is just a copy of Cobbler's data
    def get_system(self, hostname: str):
        for system in self.cobbler.get_systems():
            if system['name'] == hostname:
                return system


    def get_distro(self, name: str):
        for distro in self.cobbler.get_distros():
            if distro['name'] == name:
                return distro


    def get_profile(self, name: str):
        for profile in self.cobbler.get_profiles():
            if profile['name'] == name:
                return profile


    # returns the system handle, can be used to modify a system
    def get_system_handle(self, hostname: str):
        return self.cobbler.get_system_handle(hostname, self.token)


    # sets the profile of a system
    def set_system_profile(self, hostname: str, profile_name: str):
        handle = self.get_system_handle(hostname)

        self.cobbler.modify_system(handle, "profile", profile_name, self.token)
        self.cobbler.save_system(handle, self.token)

    # sets the kernel_options to a URL
    def set_system_args(self, hostname: str, args: List[Tuple[str, str]]):
        arg_str = ""
        for (arg_name, arg_val) in args:
            arg_str = arg_str + " " + arg_name + "=" + arg_val

        print("about to get sys_id")
        sys_id = self.get_system_handle(hostname)
        print("about to modify system")
        print("Sets system args to:", arg_str)
        self.cobbler.modify_system(sys_id, 'kernel_options', arg_str, self.token)
        print("about to save system")
        self.cobbler.save_system(sys_id, self.token)

    # sets the post install kernel args
    def set_system_post_args(self, hostname: str, url):
        sys_id = self.get_system_handle(hostname)
        self.cobbler.modify_system(sys_id, 'kernel_options_post',url, self.token)
        self.cobbler.save_system(sys_id, self.token)

    def restart_system(self, hostname: str):
        handle = self.get_system_handle(hostname)
        self.cobbler.power_system(handle, token=self.token, power='reboot')


    def sync(self):
        self.cobbler.sync(self.token)


    # adds a system to Cobbler
    # the lab ID is a string, corresponding to an imported LaaS host
    def add_system(self, system_def):
        try:
            system_def = json.loads(system_def)
            sys_id = self.cobbler.new_system(self.token)

            if not ('arch' in system_def and 'host_ports' in system_def):
                print('WARNING: Host schema for ' + system_def['server_name'] + ' is out of date! Reimport this host.')
                return None

            arch = system_def['arch']

            self.cobbler.modify_system(sys_id, 'name', system_def['server_name'], self.token)
            self.cobbler.modify_system(sys_id, 'hostname', system_def['server_name'], self.token)
            self.cobbler.modify_system(sys_id, 'profile', self.get_default_profile_name(arch), self.token)

            ifaces = {}

            for iface in system_def['host_ports'].items():
                ifaces[iface['name']] = {
                        'mac_address': iface['mac']
                    }

            self.cobbler.modify_system(sys_id, 'interfaces', ifaces, self.token)

            self.cobbler.modify_system(sys_id, 'power_address', system_def['ipmi_fqdn'], self.token)
            self.cobbler.modify_system(sys_id, 'power_user', system_def['ipmi_user'], self.token)
            self.cobbler.modify_system(sys_id, 'power_pass', system_def['ipmi_pass'], self.token)
            self.cobbler.modify_system(sys_id, 'power_mode', 'ipmitool', self.token)
            self.cobbler.modify_system(sys_id, 'netboot_enabled', True, self.token)

            self.cobbler.save_system(sys_id, self.token)
            return None
        except Exception as e:
            return e
