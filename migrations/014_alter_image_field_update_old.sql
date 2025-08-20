/*

This is just to update the current list of images in the db to support the new fields, it does not account for new rows

*/


UPDATE images
SET distro = 'Ubuntu'
WHERE name LIKE '%Ubuntu%';

UPDATE images
SET distro = 'EVE'
WHERE name LIKE '%EVE%';

UPDATE images
SET distro = 'Fedora'
WHERE name LIKE '%Fedora%';


UPDATE images
SET arch = 'x86_64'
WHERE name LIKE '%x86_64%';

UPDATE images
SET arch = 'aarch64'
WHERE name LIKE '%aarch%';


UPDATE images
SET version = '20.04 LTS'
WHERE name LIKE '%20.04 LTS%';


UPDATE images
SET version = '22.04 LTS'
WHERE name LIKE '%22.04 LTS%';


UPDATE images
SET version = '12.0.4 LTS'
WHERE name LIKE '%12.0.4-lts%';

UPDATE images
SET version = '42'
WHERE name LIKE '%42%';
