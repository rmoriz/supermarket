# Renders the mailer fixtures inside the oracle. Mirrors src/bin/mailers.rs and
# `harness/bin/mailers` diffs the two maps.
require 'json'

out = {}
base = ENV.fetch('HARNESS_BASE_URL')

version = CookbookVersion.joins(:cookbook).where(cookbooks: { name: 'redis' }).order(:created_at).last
ada = User.find(1)
render = lambda do |mail|
  m = mail.message
  { to: Array(m.to).join(','), subject: m.subject.to_s,
    text: m.text_part&.body&.to_s, html: m.html_part&.body&.to_s }
end

out['follower_notification_email'] = render.call(
  CookbookMailer.follower_notification_email(version, ada)
)

token = lambda do |name|
  ada.email_preferences.joins(:system_email).find_by(system_emails: { name: name }).token
end

out['cookbook_deleted_email'] = render.call(
  CookbookMailer.cookbook_deleted_email('redis', ada)
)

out['cookbook_deprecated_email'] = render.call(
  CookbookMailer.cookbook_deprecated_email(Cookbook.find_by(name: 'redis'), nil, ada)
)

out['deprecated_with_replacement'] = render.call(
  CookbookMailer.cookbook_deprecated_email(Cookbook.find_by(name: 'oldredis'), Cookbook.find_by(name: 'redis'), ada)
)

transfer = OwnershipTransferRequest.first
out['transfer_ownership_email'] = render.call(
  CookbookMailer.transfer_ownership_email(transfer.id)
)

collaborator = Collaborator.first
out['added_email'] = render.call(CollaboratorMailer.added_email(collaborator))

grace = User.find(2)
out['interest_email'] = render.call(
  AdoptionMailer.interest_email(Cookbook.find_by(name: 'redis').id, 'Cookbook', grace.id)
)

out['follower_email'] = render.call(
  AdoptionMailer.follower_email(Cookbook.find_by(name: 'redis').id, 'Cookbook')
)

File.write(ENV.fetch('HARNESS_OUT'), JSON.pretty_generate(out))
